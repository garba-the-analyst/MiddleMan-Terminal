import makeWASocket, {
  useMultiFileAuthState,
  fetchLatestBaileysVersion,
  DisconnectReason,
} from '@whiskeysockets/baileys';
import { Boom } from '@hapi/boom';
import Pino from 'pino';
import type { WASocket } from '@whiskeysockets/baileys';
import { config } from '../config';
import { handleInbound } from '../streams/inbound';

const logger = Pino({ level: config.LOG_LEVEL });

export let sock: WASocket | null = null;
export let pairingState: 'CONNECTED' | 'QR_PENDING' | 'LOGGED_OUT' | 'CONNECTING' = 'CONNECTING';

export function currentSocket(): WASocket {
  if (!sock) throw new Error('socket not initialized');
  return sock;
}

export async function connectToWhatsApp(): Promise<WASocket> {
  const { state, saveCreds } = await useMultiFileAuthState('auth_info_baileys');
  const { version } = await fetchLatestBaileysVersion();

  sock = makeWASocket({
    version,
    auth: state,
    printQRInTerminal: true,
    logger: logger as unknown as never,
    browser: ['MiddleMan Engine', 'Chrome', '1.0.0'],
    connectTimeoutMs: 20000,
    defaultQueryTimeoutMs: 30000,
  }) as unknown as WASocket;

  sock.ev.on('creds.update', saveCreds);

  sock.ev.on('connection.update', (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) pairingState = 'QR_PENDING';
    if (connection === 'open') {
      pairingState = 'CONNECTED';
      console.log('WhatsApp socket connected');
    }

    if (connection === 'close') {
      const code = (lastDisconnect?.error as Boom)?.output?.statusCode;
      if (code === DisconnectReason.loggedOut) {
        pairingState = 'LOGGED_OUT';
        console.error('Session logged out. Clear auth_info_baileys and re-pair via QR.');
        return;
      }
      const attempt = (update as { restartCount?: number }).restartCount ?? 2;
      const backoffMs = Math.min(60000, 2000 * 2 ** attempt);
      console.log(`Reconnecting in ${backoffMs}ms (code=${code})`);
      setTimeout(() => {
        connectToWhatsApp().catch((err) => console.error('reconnect failed', err));
      }, backoffMs);
    }
  });

  sock.ev.on('messages.upsert', async (m) => {
    if (m.type !== 'notify') return;
    for (const msg of m.messages) {
      if (msg.key.fromMe || msg.key.remoteJid === 'status@broadcast') continue;
      try {
        await handleInbound(msg);
      } catch (err) {
        logger.error({ err, id: msg.key.id ?? '?' }, 'inbound pipeline failure');
      }
    }
  });

  return sock;
}

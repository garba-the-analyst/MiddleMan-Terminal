import Redis from 'ioredis';
import { config } from '../config';
import { currentSocket, pairingState } from '../socket/connection';
import { sendSimulatedMessage } from '../anti-ban/simulate';

const redis = new Redis(config.REDIS_URL);
const GROUP = 'wa_bridge_outbound';

interface OutboundPayload {
  recipient_jid: string;
  text: string;
}

export async function drainOutboundStream(): Promise<void> {
  try {
    await redis.xgroup('CREATE', 'outbound:wa:messages', GROUP, '$', 'MKSTREAM');
  } catch (e) {
    if (!String(e).includes('BUSYGROUP')) throw e;
  }
  console.log('outbound stream consumer active');

  while (true) {
    if (pairingState !== 'CONNECTED') {
      await new Promise((r) => setTimeout(r, 2000));
      continue;
    }

    const rows = (await redis.xreadgroup(
      'GROUP',
      GROUP,
      'bridge-1',
      'COUNT',
      '5',
      'BLOCK',
      '2000',
      'STREAMS',
      'outbound:wa:messages',
      '>'
    )) as Array<[string, Array<[string, string[]]>]> | null;

    if (!rows) continue;

    for (const [, entries] of rows) {
      for (const [entryId, fields] of entries) {
        try {
          const map = new Map<string, string>();
          for (let i = 0; i < fields.length; i += 2) {
            map.set(fields[i], fields[i + 1]);
          }
          const body = JSON.parse(map.get('payload') ?? '{}') as OutboundPayload;
          await sendSimulatedMessage(currentSocket(), body.recipient_jid, body.text);
          await redis.xack('outbound:wa:messages', GROUP, entryId);
          await redis.xdel('outbound:wa:messages', entryId);
        } catch (err) {
          console.error('outbound stream failure (left unacked)', err);
        }
      }
    }
  }
}

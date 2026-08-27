import { downloadMediaMessage } from '@whiskeysockets/baileys';
import Pino from 'pino';
import type { WAMessage } from '@whiskeysockets/baileys';
import Redis from 'ioredis';
import { config, cloudinaryEnabled } from '../config';
import { currentSocket } from '../socket/connection';
import { uploadCardImage } from '../media/pipeline';

const logger = Pino({ level: config.LOG_LEVEL });
export const redis = new Redis(config.REDIS_URL, {
  lazyConnect: true,
  maxRetriesPerRequest: 3,
  enableOfflineQueue: false,
});

function extractText(m: WAMessage['message']): string {
  if (!m) return '';
  return m.conversation ?? m.extendedTextMessage?.text ?? m.imageMessage?.caption ?? '';
}

interface InboundPayload {
  message_id: string;
  sender_jid: string;
  chat_jid: string;
  text_body: string;
  has_media: boolean;
  media_url: string | null;
  media_mime: string | null;
  timestamp: number;
}

async function publish(payload: InboundPayload): Promise<void> {
  await redis.xadd(
    'inbound:wa:events',
    'MAXLEN',
    '~',
    '10000',
    '*',
    'payload',
    JSON.stringify(payload)
  );
}

export async function handleInbound(msg: WAMessage): Promise<void> {
  const startedAt = Date.now();
  const senderJid = msg.key.remoteJid ?? '';
  const hasImage = !!msg.message?.imageMessage;

  let mediaUrl: string | null = null;
  let mediaMime: string | null = null;

  if (hasImage && msg.message?.imageMessage) {
    mediaMime = msg.message.imageMessage.mimetype ?? 'image/jpeg';
    if (cloudinaryEnabled) {
      try {
        const buffer = (await downloadMediaMessage(
          msg,
          'buffer',
          {},
          { logger: logger as unknown as never, reuploadRequest: currentSocket().updateMediaMessage }
        )) as Buffer;
        mediaUrl = await uploadCardImage(buffer, String(msg.key.id));
        console.log(`media hosted: ${mediaUrl}`);
      } catch (err) {
        console.error('media pipeline failed; publishing text-only', err);
      }
    } else {
      console.warn('cloudinary disabled; image not hosted');
    }
  }

  const payload: InboundPayload = {
    message_id: String(msg.key.id),
    sender_jid: senderJid,
    chat_jid: senderJid,
    text_body: extractText(msg.message),
    has_media: !!mediaUrl,
    media_url: mediaUrl,
    media_mime: mediaMime,
    timestamp: Number(msg.messageTimestamp) || Math.floor(Date.now() / 1000),
  };

  await publish(payload);

  if (!payload.has_media && Date.now() - startedAt > 30) {
    logger.warn({ elapsed: Date.now() - startedAt, id: payload.message_id }, 'inbound ACK budget exceeded');
  }
}

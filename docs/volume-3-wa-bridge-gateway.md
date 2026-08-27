# VOLUME 3 — Gateway & Ingestion Specification (`apps/wa-bridge`)

**Version:** 2.4.0 · **Owner:** Edge Engineering · **Scope:** Baileys socket lifecycle, proxy
binding, anti-ban typing simulation, media pipeline, Redis ingestion, control-plane HTTP.

---

## 1. Architectural Overview & Technical Scope

`wa-bridge` is the ONLY component that speaks the WhatsApp protocol. It is deliberately dumb:

1. Maintain exactly one multi-device socket (`@whiskeysockets/baileys`), authenticated via a
   persisted session in `auth_info_baileys/`.
2. On inbound message: download media (if any) → upload to Cloudinary → `XADD` a compact JSON
   envelope to `inbound:wa:events` → ACK. Target end-to-end publish latency `< 30 ms`
   (media upload happens BEFORE the XADD and is not counted against the ACK budget).
3. Expose an internal-only HTTP control plane for outbound sends with human typing simulation.
4. Drain `outbound:wa:messages` as a fallback consumer when mm-api's HTTP push fails.

Non-goals: no business logic, no DB access, no AI calls.

## 2. Mathematical Formulation — Anti-Ban Typing Simulation

For outbound text `M` of length `|M|`:

```
T_delay = clamp( |M| * 35 ms + J , 1200 ms , 2500 ms ),   J ~ Uniform(-200, +300)
```

Presence lifecycle per send:

```
composing -> sleep(T_delay) -> paused -> sendMessage(text)
```

Additional rate governor (token bucket):

```
Capacity 20 tokens, refill 1 token / 3 s.
Every outbound send consumes 1 token; empty bucket delays the send by (deficit * 3 s).
```

This keeps volume under ~1 msg/3s sustained, bursty to 20 — well inside safe territory for a
single number.

## 3. Complete Implementation

### 3.1 Project layout

```
apps/wa-bridge/src/
├── index.ts            # entrypoint: express control plane + socket bootstrap
├── socket/connection.ts# lifecycle, reconnect policy, QR state
├── anti-ban/simulate.ts# presence + jitter + token bucket
├── media/pipeline.ts   # download -> temp file -> Cloudinary -> cleanup
├── streams/inbound.ts  # envelope build + XADD (<30ms path)
├── streams/outbound.ts # consumer group drainer
└── config.ts           # env parsing, fail-fast
```

### 3.2 `src/config.ts`

```typescript
import { z } from 'zod';

const Env = z.object({
  REDIS_URL: z.string().url(),
  PORT: z.coerce.number().default(3001),
  INTERNAL_API_SECRET: z.string().min(32),
  MM_API_URL: z.string().url().optional(),
  CLOUDINARY_CLOUD_NAME: z.string(),
  CLOUDINARY_API_KEY: z.string(),
  CLOUDINARY_API_SECRET: z.string(),
  WA_PROXY_URL: z.string().optional(),
});

export const config = Env.parse(process.env);
```

### 3.3 `src/socket/connection.ts`

```typescript
import makeWASocket, {
  useMultiFileAuthState,
  fetchLatestBaileysVersion,
  DisconnectReason,
} from '@whiskeysockets/baileys';
import { Boom } from '@hapi/boom';
import Pino from 'pino';
import { handleInbound } from '../streams/inbound.js';
import type { WASocket } from '@whiskeysockets/baileys';

const logger = Pino({ level: process.env.LOG_LEVEL ?? 'silent' });

export let sock: WASocket;
export let pairingState: 'CONNECTED' | 'QR_PENDING' | 'LOGGED_OUT' = 'QR_PENDING';

export async function connectToWhatsApp(): Promise<WASocket> {
  const { state, saveCreds } = await useMultiFileAuthState('auth_info_baileys');
  const { version } = await fetchLatestBaileysVersion();

  sock = makeWASocket({
    version,
    auth: state,
    printQRInTerminal: true,
    logger: logger.child({ module: 'baileys' }),
    browser: ['MiddleMan Engine', 'Chrome', '1.0.0'],
    connectTimeoutMs: 20_000,
    defaultQueryTimeoutMs: 30_000,
  }) as unknown as WASocket;

  sock.ev.on('creds.update', saveCreds);

  sock.ev.on('connection.update', (update) => {
    const { connection, lastDisconnect, qr } = update;
    if (qr) pairingState = 'QR_PENDING';
    if (connection === 'open') pairingState = 'CONNECTED';
    if (connection === 'close') {
      const code = (lastDisconnect?.error as Boom)?.output?.statusCode;
      if (code === DisconnectReason.loggedOut) {
        pairingState = 'LOGGED_OUT';
        // Session dead: require manual re-pair via /bridge/qr after clearing auth dir.
        return;
      }
      const backoffMs = Math.min(60_000, 2_000 * 2 ** (update.restartCount ?? 1));
      setTimeout(connectToWhatsApp, backoffMs);
    }
  });

  sock.ev.on('messages.upsert', async (m) => {
    if (m.type !== 'notify') return;
    for (const msg of m.messages) {
      if (msg.key.fromMe || msg.key.remoteJid === 'status@broadcast') continue;
      try {
        await handleInbound(msg);
      } catch (err) {
        logger.error({ err, id: msg.key.id }, 'inbound pipeline failure');
      }
    }
  });

  return sock;
}
```

### 3.4 `src/streams/inbound.ts`

```typescript
import { Redis } from 'ioredis';
import { downloadMediaMessage } from '@whiskeysockets/baileys';
import Pino from 'pino';
import type { WAMessage } from '@whiskeysockets/baileys';
import { uploadCardImage } from '../media/pipeline.js';
import { config } from '../config.js';

const logger = Pino({ level: 'silent' });
const redis = new Redis(config.REDIS_URL, { lazyConnect: true, maxRetriesPerRequest: 3 });
redis.connect().catch((e) => { logger.error(e); process.exit(1); });

function extractText(m: WAMessage['message']): string {
  if (!m) return '';
  return (
    m.conversation ??
    m.extendedTextMessage?.text ??
    m.imageMessage?.caption ??
    ''
  );
}

export async function handleInbound(msg: WAMessage): Promise<void> {
  const startedAt = Date.now();
  const senderJid = msg.key.remoteJid ?? '';
  const hasMedia = !!msg.message?.imageMessage;

  let mediaUrl: string | null = null;
  let mediaMime: string | null = null;

  if (hasMedia && msg.message?.imageMessage) {
    mediaMime = msg.message.imageMessage.mimeType ?? 'image/jpeg';
    try {
      const buffer = await downloadMediaMessage(
        msg,
        'buffer',
        {},
        { logger, reuploadRequest: (sock as any).updateMediaMessage }
      );
      mediaUrl = await uploadCardImage(buffer as Buffer, String(msg.key.id));
    } catch (err) {
      logger.error({ err, id: msg.key.id }, 'media pipeline failed; publishing text-only');
    }
  }

  const payload = JSON.stringify({
    message_id: String(msg.key.id),
    sender_jid: senderJid,
    chat_jid: senderJid,
    text_body: extractText(msg.message),
    has_media: !!mediaUrl,
    media_url: mediaUrl,
    media_mime: mediaMime,
    timestamp: Number(msg.messageTimestamp) || Math.floor(Date.now() / 1000),
  });

  await redis.xadd('inbound:wa:events', 'MAXLEN', '~', '10000', '*', 'payload', payload);
  const elapsed = Date.now() - startedAt;
  if (elapsed > 30 && !hasMedia) {
    logger.warn({ elapsed, id: msg.key.id }, 'inbound publish exceeded 30ms budget');
  }
}
```

### 3.5 `src/media/pipeline.ts`

```typescript
import { v2 as cloudinary } from 'cloudinary';
import fs from 'fs';
import path from 'path';
import { config } from '../config.js';

cloudinary.config({
  cloud_name: config.CLOUDINARY_CLOUD_NAME,
  api_key: config.CLOUDINARY_API_KEY,
  api_secret: config.CLOUDINARY_API_SECRET,
});

const tempDir = path.join(process.cwd(), 'temp_media');
fs.mkdirSync(tempDir, { recursive: true });

export async function uploadCardImage(buffer: Buffer, messageId: string): Promise<string> {
  const filePath = path.join(tempDir, `${messageId}.jpg`);
  try {
    await fs.promises.writeFile(filePath, buffer);
    const result = await cloudinary.uploader.upload(filePath, {
      folder: 'middleman_trades',
      public_id: messageId,
      resource_type: 'image',
    });
    return result.secure_url;
  } finally {
    await fs.promises.rm(filePath, { force: true });
  }
}
```

### 3.6 `src/anti-ban/simulate.ts`

```typescript
import type { WASocket } from '@whiskeysockets/baileys';

const BUCKET_CAPACITY = 20;
const REFILL_INTERVAL_MS = 3_000;

let tokens = BUCKET_CAPACITY;
let lastRefill = Date.now();

function refill(): void {
  const now = Date.now();
  const elapsed = Math.floor((now - lastRefill) / REFILL_INTERVAL_MS);
  if (elapsed > 0) {
    tokens = Math.min(BUCKET_CAPACITY, tokens + elapsed);
    lastRefill += elapsed * REFILL_INTERVAL_MS;
  }
}

export function computeTypingDelay(text: string): number {
  const jitter = Math.floor(Math.random() * 500 - 200); // Uniform(-200, +300)
  return Math.min(2500, Math.max(1200, text.length * 35 + jitter));
}

export async function sendSimulatedMessage(
  socket: WASocket,
  jid: string,
  text: string
): Promise<void> {
  refill();
  if (tokens <= 0) {
    await new Promise((r) => setTimeout(r, REFILL_INTERVAL_MS));
    refill();
  }
  tokens -= 1;

  const delay = computeTypingDelay(text);
  await socket.sendPresenceUpdate('composing', jid);
  await new Promise((resolve) => setTimeout(resolve, delay));
  await socket.sendPresenceUpdate('paused', jid);
  await socket.sendMessage(jid, { text });
}
```

### 3.7 `src/streams/outbound.ts`

```typescript
import { Redis } from 'ioredis';
import { sock } from '../socket/connection.js';
import { sendSimulatedMessage } from '../anti-ban/simulate.js';
import { config } from '../config.js';

const redis = new Redis(config.REDIS_URL);

interface OutboundPayload {
  recipient_jid: string;
  text: string;
  typing_delay_ms?: number;
}

export async function drainOutboundStream(): Promise<void> {
  const group = 'wa_bridge_outbound';
  try {
    await redis.xgroup('CREATE', 'outbound:wa:messages', group, '$', 'MKSTREAM');
  } catch (e: any) {
    if (!String(e.message).includes('BUSYGROUP')) throw e;
  }

  while (true) {
    const rows = (await redis.xreadgroup(
      'GROUP', group, 'bridge-1', 'COUNT', 5, 'BLOCK', 2000,
      'STREAMS', 'outbound:wa:messages', '>'
    )) as Array<[string, Array<[string, Array<[string, string]>]>]> | null;

    if (!rows) continue;

    for (const [, entries] of rows) {
      for (const [entryId, fields] of entries) {
        const map = new Map(fields.flat() as [string, string][]);
        let body: OutboundPayload;
        try {
          body = JSON.parse(map.get('payload') ?? '{}');
          await sendSimulatedMessage(sock as any, body.recipient_jid, body.text);
          await redis.xack('outbound:wa:messages', group, entryId);
          await redis.xdel('outbound:wa:messages', entryId);
        } catch (err) {
          // Leave unACKed; XAUTOCLAIM on next boot retries it.
          console.error('outbound stream failure', err);
        }
      }
    }
  }
}
```

### 3.8 `src/index.ts` — control plane + bootstrap

```typescript
import express from 'express';
import crypto from 'crypto';
import QRCode from 'qrcode-terminal';
import { config } from './config.js';
import { connectToWhatsApp, pairingState } from './socket/connection.js';
import { sendSimulatedMessage } from './anti-ban/simulate.js';
import { drainOutboundStream } from './streams/outbound.js';
import type { WASocket } from '@whiskeysockets/baileys';

const app = express();
app.use(express.json({ limit: '256kb' }));

function authGuard(req: express.Request, res: express.Response, next: express.NextFunction): void {
  const provided = req.header('X-Internal-Secret');
  const expected = config.INTERNAL_API_SECRET;
  const a = Buffer.from(provided ?? '');
  const b = Buffer.from(expected);
  if (a.length === b.length && crypto.timingSafeEqual(a, b)) return next();
  res.status(401).json({ error: 'unauthorized' });
}

app.get('/bridge/health', (_req, res) => {
  res.json({
    status: 'ok',
    pairing: pairingState,
    uptime_s: Math.floor(process.uptime()),
    rss_mb: Math.round(process.memoryUsage().rss / 1024 / 1024),
  });
});

app.post('/bridge/send-message', authGuard, async (req, res) => {
  const { recipient_jid, text } = req.body ?? {};
  if (!recipient_jid || !text || typeof text !== 'string' || text.length > 4096) {
    res.status(422).json({ error: 'recipient_jid and text (<=4096 chars) required' });
    return;
  }
  try {
    await sendSimulatedMessage(sock as unknown as WASocket, recipient_jid, text.slice(0, 4000));
    res.json({ status: 'sent' });
  } catch (err: any) {
    res.status(502).json({ error: err.message });
  }
});

app.post('/bridge/qr', authGuard, async (_req, res) => {
  if (pairingState === 'CONNECTED') {
    res.json({ status: 'already_connected' });
    return;
  }
  QRCode.toString('re-scan-required', { type: 'terminal' }, (_e, out) => process.stdout.write(out));
  res.json({ status: pairingState });
});

app.listen(config.PORT, () => {
  console.log(`Bridge control plane on :${config.PORT}`);
});

connectToWhatsApp();
drainOutboundStream();

process.on('SIGTERM', () => {
  console.log('SIGTERM: flushing and closing');
  process.exit(0);
});
```

### 3.9 `apps/wa-bridge/package.json` (target)

```json
{
  "name": "wa-bridge",
  "version": "2.4.0",
  "private": true,
  "scripts": {
    "dev": "tsx watch src/index.ts",
    "build": "tsc -p tsconfig.json",
    "start": "node dist/index.js",
    "test": "vitest run"
  },
  "dependencies": {
    "@whiskeysockets/baileys": "^6.7.13",
    "cloudinary": "^2.10.0",
    "dotenv": "^17.0.0",
    "express": "^5.2.1",
    "ioredis": "^5.4.1",
    "pino": "^10.3.1",
    "qrcode-terminal": "^0.12.0",
    "zod": "^3.23.8"
  },
  "devDependencies": {
    "@types/express": "^5.0.0",
    "@types/node": "^22.0.0",
    "@types/qrcode-terminal": "^0.12.0",
    "tsx": "^4.19.0",
    "typescript": "^5.5.0",
    "vitest": "^2.0.0"
  }
}
```

`tsconfig.json`: `module: NodeNext`, `outDir: dist`, `strict: true`.

## 4. Data Schemas & Structural Interfaces

See Vol 1 §1.2/§1.3 for the exact stream envelopes; this service owns serialization for both.
Control-plane responses:

| Route | Success | Errors |
|-------|---------|--------|
| `GET /bridge/health` | `{status,pairing,uptime_s,rss_mb}` | — |
| `POST /bridge/send-message` | `{status:"sent"}` | 401 secret, 422 validation, 502 WhatsApp |
| `POST /bridge/qr` | `{status}` | 401 secret |

## 5. Error Handling, Retry & Edge Cases

| Case | Behavior |
|------|----------|
| Cloudinary down during media ingest | Publish event with `has_media:false`; mm-api prompts user to re-send image |
| Redis down at boot | Process exits(1); Docker restarts until Redis healthy (`depends_on` gate) |
| Baileys 401/conflict | Exponential reconnect up to 60 s cap; `loggedOut` stops auto-reconnect |
| Duplicate message key (WhatsApp redelivery) | Harmless: mm-api dedupes via `processed_messages` |
| Send to non-existent JID | 502 surfaced to caller; outbound falls into stream retry path |
| Memory guard | If RSS > 200 MB, bridge logs CRITICAL and exits (Docker restart clears heap) |

## 6. Verification Test Cases & Command Sequences

```bash
# V3-T1: unit tests for jitter bounds & bucket
npm test                       # vitest: computeTypingDelay in [1200,2500]; bucket refills

# V3-T2: ACK budget
redis-cli --latency-history -h localhost &
# send a text-only WhatsApp message; assert no 'exceeded 30ms budget' warnings in logs

# V3-T3: end-to-end echo
redis-cli XADD inbound:wa:events '*' payload '{"message_id":"e1","sender_jid":"<you>@s.whatsapp.net","text_body":"ping","has_media":false,"timestamp":0}'
# expect reply consumed by dev mm-api stub

# V3-T4: control-plane auth
curl -s -X POST localhost:3001/bridge/send-message -H 'Content-Type: application/json' \
  -d '{"recipient_jid":"x","text":"hi"}'          # expect 401
curl -s -X POST localhost:3001/bridge/send-message \
  -H "X-Internal-Secret: $INTERNAL_API_SECRET" -H 'Content-Type: application/json' \
  -d "{\"recipient_jid\":\"<jid>\",\"text\":\"integration probe\"}"   # expect {"status":"sent"}

# V3-T5: media path
# send an image on WhatsApp -> object appears in Cloudinary folder middleman_trades,
# stream event carries https URL, temp_media/ is empty afterwards

# V3-T6: crash resilience
docker kill middleman-wa-bridge-1 && docker compose up -d wa-bridge
# session resumes without re-pairing (auth_info_baileys volume intact)
```

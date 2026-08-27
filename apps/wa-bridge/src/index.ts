import express from 'express';
import crypto from 'crypto';
import { config } from './config';
import { connectToWhatsApp, currentSocket, pairingState } from './socket/connection';
import { sendSimulatedMessage } from './anti-ban/simulate';
import { drainOutboundStream } from './streams/outbound';
import type { WASocket } from '@whiskeysockets/baileys';

const app = express();
app.use(express.json({ limit: '256kb' }));

function authGuard(
  req: express.Request,
  res: express.Response,
  next: express.NextFunction
): void {
  const provided = req.header('X-Internal-Secret') ?? '';
  const expected = config.INTERNAL_API_SECRET;
  const a = Buffer.from(provided);
  const b = Buffer.from(expected);
  if (a.length === b.length && crypto.timingSafeEqual(a, b)) {
    next();
    return;
  }
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
  if (
    typeof recipient_jid !== 'string' ||
    !recipient_jid.includes('@') ||
    typeof text !== 'string' ||
    text.length > 4096
  ) {
    res.status(422).json({ error: 'recipient_jid and text (<=4096 chars) required' });
    return;
  }
  try {
    await sendSimulatedMessage(currentSocket(), recipient_jid, text.slice(0, 4000));
    res.json({ status: 'sent' });
  } catch (err) {
    res.status(502).json({ error: err instanceof Error ? err.message : String(err) });
  }
});

app.post('/bridge/qr', authGuard, (_req, res) => {
  res.json({
    status: pairingState,
    hint:
      pairingState === 'LOGGED_OUT'
        ? 'clear auth_info_baileys volume and restart the container to re-pair'
        : 'scan the terminal QR on next boot if pairing is pending',
  });
});

app.listen(config.PORT, () => {
  console.log(`Bridge control plane on :${config.PORT}`);
});

connectToWhatsApp()
  .then(() => drainOutboundStream())
  .catch((err) => {
    console.error('fatal bootstrap failure', err);
    process.exit(1);
  });

process.on('SIGTERM', () => {
  console.log('SIGTERM received; shutting down');
  process.exit(0);
});

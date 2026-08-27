import type { WASocket } from '@whiskeysockets/baileys';

const BUCKET_CAPACITY = 20;
const REFILL_INTERVAL_MS = 3000;

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
  const jitter = Math.floor(Math.random() * 500 - 200);
  return Math.min(2500, Math.max(1200, text.length * 35 + jitter));
}

export async function sendSimulatedMessage(
  socket: WASocket,
  jid: string,
  text: string
): Promise<void> {
  refill();
  if (tokens <= 0) {
    await new Promise((resolve) => setTimeout(resolve, REFILL_INTERVAL_MS));
    refill();
  }
  tokens -= 1;

  await socket.sendPresenceUpdate('composing', jid);
  await new Promise((resolve) => setTimeout(resolve, computeTypingDelay(text)));
  await socket.sendPresenceUpdate('paused', jid);
  await socket.sendMessage(jid, { text });
}

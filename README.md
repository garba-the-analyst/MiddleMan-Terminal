# MiddleMan

WhatsApp-native neo-bank: gift-card liquidation, crypto terminal, P2P transfers.
Architecture spec lives in [`docs/`](docs/README.md) (v2.4.0, Volumes 0–Iota).

## Stack

- `crates/mm-api` — Axum HTTP plane + Redis Streams worker (FSM, NLU, ledger)
- `apps/wa-bridge` — Baileys WhatsApp gateway (anti-ban typing simulation, media pipeline)
- `crates/mm-vault` — Argon2id PINs + AES-256-GCM key vault (zeroize, AAD-bound envelopes)
- `crates/mm-db` — SQLx compile-time-checked query layer
- Postgres 16 + Redis 7 via Docker Compose

## Quickstart (dev)

```bash
# 1. Infrastructure (postgres :5434, redis :6379) — fresh volume + migrations
bash scripts/reset-dev-db.sh

# 2. Core engine (.env already carries DATABASE_URL/secrets)
MM_AUTO_MIGRATE=false cargo run -p mm-api        # listens on :3000

# 3. WhatsApp gateway
cd apps/wa-bridge && npm install && npm run dev   # listens on :3001, scan QR once
```

## Smoke test without WhatsApp

```bash
source <(grep -E '^(ADMIN_API_TOKEN|INTERNAL_API_SECRET)=' .env)

curl -s localhost:3000/api/v1/admin/health

curl -s -X POST localhost:3000/api/v1/debug/ingest \
  -H "X-Internal-Secret: $INTERNAL_API_SECRET" -H 'Content-Type: application/json' \
  -d '{"message_id":"t1","sender_jid":"2348012345678@s.whatsapp.net",
       "text_body":"I wan sell $50 Steam card"}'

curl -s localhost:3000/api/v1/admin/dashboard -H "Authorization: Bearer $ADMIN_API_TOKEN"
```

## Tests

```bash
DATABASE_URL=postgres://mm_user:mm_password@localhost:5434/middleman_db cargo test --workspace
cd apps/wa-bridge && npm run typecheck
```

## Layout notes

- Inbound path: bridge → `XADD inbound:wa:events` → worker group `mm_api_workers` → FSM → reply
  (HTTP push to bridge primary, `outbound:wa:messages` stream fallback).
- Idempotency: every event claims its `message_id` in `processed_messages` before processing.
- Admin routes use a bearer token (`ADMIN_API_TOKEN`) until JWT auth lands (Day 8).

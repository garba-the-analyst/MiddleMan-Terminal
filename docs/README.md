# MIDDLEMAN — Architectural Specification Suite (v2.4.0)

Institutional-grade, multi-tenant financial micro-engine and conversational neo-bank operating
inside WhatsApp. Self-hosted on a `<= 1 GB RAM` VPS. Meta Cloud API bypassed via a self-hosted
Baileys bridge during MVP; drop-in Cloud API adapter deferred to Phase 3.

## Reading Order

| # | Document | Volume | Owns | Repo Target |
|---|----------|--------|------|-------------|
| 1 | [volume-0-devops-infrastructure.md](volume-0-devops-infrastructure.md) | Vol 0 | VPS sizing, containers, Caddy, hardening, deploy runbook | `docker-compose.yml`, `Caddyfile`, Dockerfiles |
| 2 | [volume-1-system-architecture-fsm.md](volume-1-system-architecture-fsm.md) | Vol 1 | Redis Streams topology, FSM, idempotency, DLQ doctrine | `crates/mm-api/src/fsm/`, `crates/mm-core/` |
| 3 | [volume-2-database-schema.md](volume-2-database-schema.md) | Vol 2 | Full DDL, ledger invariants, sqlx compile-time contracts | `migrations/`, `crates/mm-db/` |
| 4 | [volume-3-wa-bridge-gateway.md](volume-3-wa-bridge-gateway.md) | Vol 3 | Baileys socket lifecycle, anti-ban simulation, media pipeline | `apps/wa-bridge/` |
| 5 | [volume-alpha-vault-cryptography.md](volume-alpha-vault-cryptography.md) | Vol Alpha | Argon2id PINs, AES-256-GCM vault, zeroize discipline | `crates/mm-vault/` |
| 6 | [volume-beta-nlu-engine.md](volume-beta-nlu-engine.md) | Vol Beta | Gemini NLU, strict JSON schema, Pidgin fallback rulebook | `crates/mm-ai/` |
| 7 | [volume-gamma-fiat-banking.md](volume-gamma-fiat-banking.md) | Vol Gamma | Flutterwave NUBANs, webhook verification, Yellow Card FX quotes | `crates/mm-fiat/` |
| 8 | [volume-delta-crypto-trading.md](volume-delta-crypto-trading.md) | Vol Delta | Wallet derivation, Jupiter v6 swaps, GoPlus radar, P2P ledger | `crates/mm-crypto/` |
| 9 | [volume-epsilon-gift-card-engine.md](volume-epsilon-gift-card-engine.md) | Vol Epsilon | Card ingestion → OCR → admin desk → atomic NGN settlement | `crates/mm-api/src/handlers/` |
| 10 | [volume-zeta-admin-dashboard.md](volume-zeta-admin-dashboard.md) | Vol Zeta | Vue 3 + Pinia ops panel, WebSocket live desk | `apps/admin-dashboard/` |
| 11 | [volume-eta-notifications-antifraud.md](volume-eta-notifications-antifraud.md) | Vol Eta | SMTP alerts, velocity checks, PIN strikes, withdrawal guard | `crates/mm-notifications/` |
| 12 | [volume-iota-mvp-build-plan.md](volume-iota-mvp-build-plan.md) | Vol Iota | 14-day sprint map, acceptance criteria, launch checklist | — |

## Canonical Laws (apply to every volume)

1. **Decoupled Ingestion** — `wa-bridge` ACKs inbound events into Redis Streams (`inbound:wa:events`) in `< 30 ms`. All business logic runs in `mm-api` consumers.
2. **Zero In-Memory Keys** — Private keys exist in plaintext RAM only inside a signing scope, wrapped by `zeroize::ZeroizeOnDrop`. At rest: AES-256-GCM.
3. **Deterministic State** — AI performs intent parsing ONLY. Every ledger mutation is an SQLx-checked PostgreSQL statement inside a transaction.
4. **Resilient Asynchrony** — Tokio tasks + exponential backoff + dead-letter queues. No synchronous user-facing waits on external APIs beyond quote latency.
5. **Anti-Ban Integrity** — Human typing jitter (1.2 s–2.5 s), presence simulation, inbound-driven outbound rate limits.
6. **Low Footprint** — Total container memory `<= 1000 MB` on 1 vCPU.

## Environment Contract

All volumes assume the following environment variables (see `.env.example`):

```
DATABASE_URL=postgres://...            # Neon or local Postgres
REDIS_URL=redis://redis:6379           # Redis 7
MM_MASTER_KEY=<64 hex chars>           # Decodes to 32-byte AES-256-GCM master key
JWT_SECRET=<random 48+ chars>          # Admin dashboard sessions
INTERNAL_API_SECRET=<random 32+ chars> # wa-bridge <-> mm-api shared secret
GEMINI_API_KEY=...                     # mm-ai NLU
FLUTTERWAVE_SECRET_KEY=...             # mm-fiat
FLW_WEBHOOK_HASH=...                   # verif-hash header secret
CLOUDINARY_CLOUD_NAME / _API_KEY / _API_SECRET  # gift card image hosting
SMTP_HOST / SMTP_USER / SMTP_PASS      # mm-notifications
ADMIN_ALERT_EMAIL=ops@middleman.africa
```

## Implementation Status vs. Spec (updated after Foundation build)

| Component | Status | Notes |
|-----------|--------|-------|
| `migrations/` | **Done** | Full Vol 2 DDL (12 tables) + catalogue seed; `scripts/reset-dev-db.sh` provisions clean dev DB |
| `crates/mm-core` | **Done** | FlowState enum, transition table, domain errors — unit tested |
| `crates/mm-db` | **Done** | sqlx-macro query layer: users, idempotency claims, catalogue rates, atomic trade settlement |
| `crates/mm-vault` | **Done** | HKDF subkeys, versioned AAD envelopes, zeroize buffers, spec Argon2id params — 14 tests |
| `crates/mm-ai` | **Done** | Strict JSON schema Gemini client + deterministic Pidgin rulebook fallback — 11 tests |
| `crates/mm-api` | **Core done** | Worker consumes `inbound:wa:events` (group `mm_api_workers`), FSM advance, outbound HTTP+stream fallback, admin dashboard/resolve with bearer auth, debug ingest endpoint. E2E verified incl. double-approval race |
| `apps/wa-bridge` | **Core done** | Redis publisher (<30 ms ACK path), typing simulation + token bucket, Cloudinary media, control plane w/ shared-secret guard, outbound stream consumer. TypeScript compiles; QR pairing pending first live run |
| Admin dashboard (Vue) | Starter only | Vol Zeta views not yet built; REST API is ready behind bearer token |
| `mm-fiat`, `mm-crypto`, `mm-notifications` | Stub | Days 7–12 of Vol Iota |

Each volume document is the authoritative target state. Where the repo deviates, the volume's
"Verification" section defines the command sequence that must pass before the volume is marked done.

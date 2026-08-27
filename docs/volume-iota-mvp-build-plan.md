# VOLUME IOTA — Rapid MVP 14-Day Fast-Track Build Plan

**Version:** 2.4.0 · **Owner:** Program Management · **Scope:** Sequenced delivery map with
acceptance criteria per phase, risk register, launch checklist.

---

## 1. Scope Definition (Integrated Fast-Track MVP)

1. **WhatsApp Gateway (`apps/wa-bridge`)** — self-hosted Baileys bridge: Redis Streams ingestion,
   typing simulation, Cloudinary media path, control plane. *(Vol 3)*
2. **Core Engine (`mm-api` + `mm-db` + `mm-core`)** — registration, FSM state handling,
   Neon PostgreSQL persistence via migrations + sqlx offline contracts. *(Vol 1, 2)*
3. **Gift Card Liquidation** — OCR ingestion, pending queue, Vue admin desk, atomic NGN credit.
   *(Vol Epsilon, Zeta)*
4. **Fiat Wallets (NUBAN)** — Flutterwave virtual account issuance + webhook top-ups.
   *(Vol Gamma)*
5. **Degen Crypto & P2P** — Solana address provisioning at onboarding; internal phone-number P2P;
   Jupiter swap execution from raw command. *(Vol Delta)*

Explicitly OUT of MVP scope: perps (`active_positions` table ships empty), Tron/TON custody,
EVM swaps (radar read-only only), Yellow Card live FX (static catalogue rates used instead),
KYC Tier-2 review UI (table exists, desk view Phase 2).

## 2. Sprint Schedule

```
Days 1-3   [FOUNDATION]     Monorepo alignment, migrations v2, Redis Streams FSM skeleton, bridge rewrite
Days 4-6   [VAULT & NLU]    mm-vault hardening (zeroize/AAD), NLU schema + fallback rulebook, FSM wiring
Days 7-9   [GIFT CARD]      Vision OCR, trade lifecycle, admin auth + Trade Desk, resolve settlement
Days 10-12 [CRYPTO & P2P]   Key provisioning, Jupiter swap pipeline, GoPlus gate, P2P ledger + velocity guard
Days 13-14 [DEPLOYMENT]     Compose prod profile, Caddy TLS, VPS hardening, launch rehearsal + go-live
```

### Day-by-day acceptance gates

| Day | Deliverable | Acceptance Test |
|-----|-------------|-----------------|
| 1 | Migrations replace inline DDL; `.sqlx` committed | `sqlx migrate run` on clean DB; `cargo sqlx prepare --check` green in CI |
| 2 | Bridge → Redis Streams; consumer group drains | Vol 3 §6 T2/T3 pass; ACK budget warnings silent for text msgs |
| 3 | FSM engine + idempotency table | Vol 1 §6 T1–T3 pass; duplicate replay produces single ledger row |
| 4 | Argon2id params enforced + AAD envelope | Vol Alpha §6 T1/T2 green |
| 5 | NLU strict schema + rulebook fallback | Vol Beta §6 T1/T2; bogus API key degrades to rulebook mode |
| 6 | PIN set/verify flows end-to-end in chat | Wrong-PIN strikes lock account after 3rd attempt (Vol Eta T6) |
| 7 | Vision OCR + trade creation | Vol Epsilon §6 T2 fixture math exact |
| 8 | Admin JWT + dashboard data endpoints | Login flow; 401s without token (Vol Zeta T2) |
| 9 | Resolve → atomic credit → WhatsApp notify | Double-click race yields single payout (Vol Epsilon T3) |
| 10 | Solana provisioning on first `/balance` | `key_vaults` row decryptable; public key matches derivation test |
| 11 | Jupiter swap command live on devnet→mainnet flag | Route-deviation guard blocks tampered quotes (Vol Delta T2) |
| 12 | P2P transfers + velocity guard | Concurrent transfer test keeps balances ≥ 0 (Vol Delta T5) |
| 13 | Prod compose + Caddy + provisioning script | Vol 0 §6 T1–T5 all pass on droplet |
| 14 | Launch rehearsal: kill/restore drills, DLQ drain drill, QR re-pair drill | All volumes' verification suites re-run clean; ops runbook printed |

## 3. Risk Register & Mitigations

| # | Risk | P×I | Mitigation |
|---|------|-----|------------|
| R1 | WhatsApp number banned during testing | H×H | Typing sim from day 2; no broadcast; ≤20 msg burst cap; warm-up: human conversation on the number for 72 h before automation |
| R2 | Gemini outage during desk hours | M×M | Rulebook fallback + manual trade creation path (agent enters brand/value) |
| R3 | Neon cold-start latency > 5 s | M×L | Pool warm ping every 30 s; FSM replies "syncing" rather than hanging |
| R4 | Agent double-payout race | L×H | DB-gated status flip (Vol 2 §4.3) — structurally impossible |
| R5 | Master key loss | L×H | Printed sealed-envelope procedure; key stored in two physical locations; rotation drill day 14 |
| R6 | Baileys breaking change mid-sprint | M×M | Lock baileys version; smoke suite runs on every deploy |
| R7 | 1 GB OOM under message burst | M×M | Hard container limits (Vol 0); Redis MAXLEN trims; load test day 13 |

## 4. Launch Checklist (Day 14)

```
[ ] All volume verification suites pass (V0..VEta)
[ ] .env secrets present, chmod 600, backed up to vault (never git)
[ ] MM_MASTER_KEY escrowed physically; JWT_SECRET + INTERNAL_API_SECRET unique randoms
[ ] Ops number paired; session volume backed up (encrypted tar to offsite)
[ ] price_catalogue populated with today's real rates; fallback floor reviewed
[ ] Flutterwave sandbox -> live keys swapped; webhook URL registered + verif-hash set
[ ] Solana mainnet RPC provider configured; TREASURY_WALLET funded with gas float
[ ] Caddy TLS issuing for production domain; HSTS preload submitted
[ ] fail2ban + ufw active; nmap sweep shows only 22/80/443
[ ] docker stats snapshot < 900 MB total under synthetic load
[ ] Admin desk credentials created; SUPER_ADMIN password rotated from seed
[ ] DLQ empty; alert email inbox receiving; ops WhatsApp JID receiving
[ ] Incident runbook laminated: ban recovery, key rotation, DB restore drill timestamps
```

## 5. Post-MVP Backlog (Phase 3 seeds)

1. Meta Cloud API adapter behind the same Redis contract (`crates/mm-api/src/gateways/meta.rs`).
2. dYdX v4 / Aevo perps module feeding `active_positions`.
3. LI.FI EVM swaps + full EVM custody.
4. Yellow Card live FX loop replacing static catalogue spreads.
5. KYC Tier-2 document desk + Dojah integration.
6. Multi-number bridge pool (one socket per line, shared stream namespace).

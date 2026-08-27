# MiddleMan — Competition Slides Outline
# Track: 02 AI Software and Website Dev (also fits 01 AI Automation)
# Time: 10-12 slides, 5-min video walkthrough

## Slide 1: Title
**MiddleMan — WhatsApp-native AI Neo-Bank**
Tagline: Trade gift cards, check balances, and move money without ever leaving WhatsApp.
Team / Repo: github.com/garba-the-analyst/MiddleMan-Terminal

## Slide 2: Problem
- Nigeria: 109M WhatsApp users, but gift-card liquidation is manual, slow, and scam-prone.
- Existing bots use Meta Cloud API (expensive, slow approval, template-locked) or are unbanked.
- Users speak Pidgin/English mixed — bots fail on "I wan sell $100 Apple card" or "wetin dey my balance".

## Slide 3: Solution
- **MiddleMan** — institutional-grade conversational neo-bank inside WhatsApp (Baileys bridge, no Meta fees at MVP).
- 4 verticals in one chat: gift-card liquidation, non-custodial wallets (EVM/Solana), DEX swaps, zero-gas P2P by phone number.
- Human-in-the-loop desk + AI handles the rest.

## Slide 4: AI Superpower (What Judges See in Video)
1. **NLU Engine (`crates/mm-ai`)** — Gemini Flash + deterministic Pidgin fallback. Parses "50k Steam card to cash" → `{intent: LIQUIDATE_GIFT_CARD, brand: STEAM, amount: 50000}`. Confidence <0.6 → help menu. No hallucination: all ledger writes are SQLx.
2. **Vision OCR (planned)** — card photo → brand/value/code → catalogue rate.
3. **Anti-fraud** — velocity check (>3 P2P/60s → 15-min cooldown), high-value hold (>₦500k → manual approval), PIN strike lock (3 fails → 2h).

## Slide 5: Architecture (1 GB VPS)
- `wa-bridge` (Node/Baileys) → Redis Streams `inbound:wa:events` (<30ms ACK) → `mm-api` (Rust/Axum workers) → Neon Postgres + Redis.
- Outbound: HTTP push to bridge (typing jitter 1.2–2.5s) + stream fallback.
- Doctrine: Decoupled ingestion, zero in-memory keys (Argon2id + AES-256-GCM + zeroize), deterministic state (AI = parsing only).

## Slide 6: Demo Flow (What Video Shows)
1. WhatsApp: "I wan sell $100 Apple card" + photo → AI → quote ₦150,000 (catalogue 1500/$)
2. Dashboard: `http://localhost:5173` — pending card appears live (poll 3s), Inspect Image, Approve & Pay
3. Wallet: `psql` shows +₦150,000, transaction `GIFT_CARD_PAYOUT` SUCCESS, user gets WhatsApp confirmation
4. Second message: "wetin dey my balance" → AI → "Available balance: ₦150,000" (no trade)
5. Idempotency: replay same message_id → no duplicate trade. Double-approve race → single credit (live test).

## Slide 7: Tools Used
- Rust (Tokio/Axum/SQLx), Node (Baileys), Vue 3 + Vite + Tailwind, Postgres (Neon), Redis 7, Docker/Caddy
- AI: Gemini Flash (structured JSON), regex fallback, GOPlus radar (planned), Jupiter v6 (planned)
- Infra: 1 vCPU / 1 GB VPS, static musl binary (~8MB), 5 containers ≤1GB total

## Slide 8: Business Impact
- Instant liquidity for gift cards at transparent catalogue rates (no hidden spreads).
- Zero-gas internal P2P by phone number → financial inclusion.
- Desk throughput: <2s per approval, audit-logged, double-credit impossible (DB-gated).
- Path to revenue: 1.5–3% FX spread, 0.5% swap fee, per-trade desk margin.

## Slide 9: What's Working Today (for Reviewers)
- `bash scripts/reset-dev-db.sh && cargo run -p mm-api` → `:3000` + worker
- `npm run dev` in `apps/admin-dashboard` → `:5173` with live Trades + Catalogue
- `bash scripts/demo-video.sh` → one-click seed + approval (see repo README)
- Tests: 34 Rust tests (vault 14, ai 11, core 6) + E2E (double-approval, idempotency)

## Slide 10: Roadmap (Next 14 Days — Volume Iota)
- Days 7–9: Vision OCR + JWT auth + perps positions
- Days 10–12: Jupiter swaps + GoPlus gate + P2P velocity guard
- Days 13–14: Caddy TLS + 1 GB hardening → production

## Slide 11: Live Links (for Drive Folder)
- GitHub: github.com/garba-the-analyst/MiddleMan-Terminal (commit `335a2f1` + demo fixes)
- Live Demo: run locally per README Quickstart, or deployed dashboard URL (add after deploy)
- Video: <5 min screen record of flow in Slide 6 (terminal + browser)

## Slide 12: Ask
We are building the WhatsApp bank Nigerians already live in — AI makes it speak their language and move their money safely.

---
# Video Script (4:30, no cuts)
0:00 Title + problem (30s)
0:30 Architecture diagram (30s)
1:00 Terminal: `bash scripts/demo-video.sh` — show health, 3 ingests, 2 trades created, 1 balance check ignored (60s)
2:00 Browser: http://localhost:5173 — Pending Review cards, Inspect Image, Approve & Pay, wallet update in DB (60s)
3:00 Terminal: `psql` wallet + ` Approve` race (single credit) + idempotency replay (30s)
3:30 Code: `crates/mm-ai/src/fallback.rs` + `crates/mm-vault/src/aead.rs` (15s)
3:45 Tools + impact (30s)
4:15 Live links + repo (15s)

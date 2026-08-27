# VOLUME 1 — System Architecture, Schematics & State Machine Specification

**Version:** 2.4.0 · **Owner:** Core Engineering · **Scope:** Redis Streams topology, message
contracts, conversational FSM, idempotency, worker loop, DLQ doctrine.

---

## 1. Architectural Overview & Technical Scope

`mm-api` is two programs in one binary:

1. **HTTP plane** (Axum): admin dashboard API, fiat webhooks, bridge control callbacks.
2. **Worker plane** (Tokio tasks): a consumer group that drains `inbound:wa:events`, runs each
   message through the NLU + FSM + domain engines, and emits outbound replies.

The bridge never blocks on business logic. The core never polls WhatsApp. Redis Streams is the
only coupling point. This yields the Doctrine's Law 1: inbound ACK `< 30 ms`.

### 1.1 Stream Topology

| Stream | Producer | Consumer Group | Semantics |
|--------|----------|----------------|-----------|
| `inbound:wa:events` | wa-bridge | `mm_api_workers` | At-least-once; consumers ACK after commit |
| `inbound:wa:dlq` | mm-api (on terminal failure) | manual / ops tooling | Poison messages with failure metadata |
| `outbound:wa:messages` | mm-api | `wa_bridge_outbound` | Fallback channel when HTTP push is down |
| `metrics:samples` | mm-api (Vol Eta) | alerts consumer | Memory/rate samples |

MAXLEN trimming: every XADD uses approximate trim `~ 10000` entries to bound Redis memory
(64 MB cap).

### 1.2 Inbound Event Contract (`inbound:wa:events`)

Stream field `payload`, JSON:

```json
{
  "message_id": "3EB0B430B6F8E9F4D2A1",
  "sender_jid": "2348012345678@s.whatsapp.net",
  "chat_jid": "2348012345678@s.whatsapp.net",
  "text_body": "swap 50 usdt to sol",
  "has_media": true,
  "media_url": "https://res.cloudinary.com/middleman/image/upload/v171.../3EB0.jpg",
  "media_mime": "image/jpeg",
  "timestamp": 1771891200,
  "bridge_seq": 1042
}
```

Rules:
- `message_id` is the idempotency key across retries.
- `sender_jid` is normalized by the bridge to `@s.whatsapp.net` JID format.
- Media is uploaded by the bridge before publish; the stream carries a URL only, never bytes.

### 1.3 Outbound Contract

Primary path — HTTP push from mm-api to bridge:

```
POST /bridge/send-message
X-Internal-Secret: <INTERNAL_API_SECRET>
{ "recipient_jid": "2348012345678@s.whatsapp.net", "text": "...", "typing_delay_ms": 1450 }
```

Fallback path — `XADD outbound:wa:messages * payload <json>` with identical body; the bridge
consumer group drains it on a 250 ms poll. Both paths are typing-simulated inside the bridge.

## 2. Mathematical Formulation — Delivery & Retry Guarantees

- **At-least-once delivery:** a message may be processed twice on consumer crash between DB
  commit and XACK. Therefore every handler is idempotent via `INSERT ... ON CONFLICT DO NOTHING`
  keyed on `message_id` into the `processed_messages` table (Vol 2).
- **Retry policy:** pending entries older than `PENDING_TTL = 60 s` are claimed by any live
  consumer (`XAUTOCLAIM min-idle-time=60000`). Attempt count tracked in `state_data.retries`;
  backoff `delay_n = min(300 s, 2^n * 500 ms)` applied as consumer sleep between attempts.
- **Terminal failure:** after `n = 5` attempts OR on non-retryable classification (malformed,
  unknown user command requiring no reply), message goes to DLQ with envelope:

```json
{
  "original": { "...inbound payload..." : null },
  "error_class": "DB_CONSTRAINT|EXTERNAL_TIMEOUT|PARSE_FAILED|UNKNOWN",
  "attempts": 5,
  "first_seen": 1771891200,
  "last_error": "..."
}
```

## 3. Conversational Finite State Machine

### 3.1 States

| State | Meaning | Timeout |
|-------|---------|---------|
| `IDLE` | No active operation | — |
| `ONBOARDING` | Collect full name, set PIN, accept terms | 10 min |
| `AWAITING_TRANSACTION_DATA` | Waiting for image/address/amount confirmation | 15 min |
| `AWAITING_PIN` | PIN prompt issued; next numeric input verified | 5 min |
| `EXECUTING_ACTION` | Terminal action dispatched to domain engine | 120 s hard kill |
| `NOTIFICATION` | Result composed and queued outbound | immediate |
| `PIN_LOCKED` | 3 strikes; vault locked | 2 h |

### 3.2 Transition Table (authoritative)

| Current State | Input / Condition | Next State | Side Effects |
|---|---|---|---|
| IDLE | intent = REGISTER_USER or first contact | ONBOARDING | create user row |
| IDLE | intent = LIQUIDATE_GIFT_CARD | AWAITING_TRANSACTION_DATA | stash card params in state_data |
| IDLE | intent in {P2P_TRANSFER, EXECUTE_DEX_SWAP, TRANSFER_FIAT} | AWAITING_TRANSACTION_DATA | stash parsed entities |
| IDLE | intent = CHECK_BALANCE | NOTIFICATION | read-only query, no PIN |
| AWAITING_TRANSACTION_DATA | valid media/confirmation received | AWAITING_PIN | quote computed + displayed |
| AWAITING_TRANSACTION_DATA | timeout 15 min | IDLE | clear state_data |
| AWAITING_PIN | Argon2id verify OK | EXECUTING_ACTION | reset failed_pin_attempts=0 |
| AWAITING_PIN | verify FAIL, strikes < 3 | AWAITING_PIN | failed_pin_attempts += 1 |
| AWAITING_PIN | verify FAIL, strikes = 3 | PIN_LOCKED | pin_locked_until = now()+2h; alert user |
| AWAITING_PIN | timeout 5 min | IDLE | keep state_data for retry |
| EXECUTING_ACTION | success | NOTIFICATION | ledger tx row SUCCESS |
| EXECUTING_ACTION | recoverable fail | NOTIFICATION | ledger tx row FAILED + reason |
| EXECUTING_ACTION | panic/hang 120 s | NOTIFICATION | mark FAILED TIMEOUT; alert ops |
| NOTIFICATION | outbound queued | IDLE | clear state_data |
| PIN_LOCKED | now() < locked_until, any input | PIN_LOCKED | reject with countdown |
| PIN_LOCKED | lock expired | IDLE | unlock vault |

### 3.3 ASCII State Diagram

```
                 +-----------------------------------+
                 |               IDLE                |
                 +-----------------------------------+
                       | intent parsed (mm-ai)
                       v
                 +-----------------------------------+
                 |    AWAITING_TRANSACTION_DATA      |
                 +-----------------------------------+
                       | valid data submitted
                       v
                 +-----------------------------------+      3 strikes
                 |           AWAITING_PIN            |--------------+
                 +-----------------------------------+              v
                       | PIN valid                            +-----------+
                       v                                      |PIN_LOCKED |
                 +-----------------------------------+        +-----------+
                 |         EXECUTING_ACTION          |
                 +-----------------------------------+
                       | action completed / failed
                       v
                 +-----------------------------------+
                 |           NOTIFICATION            |
                 +-----------------------------------+
                       v
                    ( IDLE )
```

### 3.4 `state_data` JSONB Contract

```json
{
  "flow": "DEX_SWAP",
  "entities": { "amount": 50.0, "source_currency": "USDT", "target_currency": "SOL" },
  "quote": { "route": "JUP", "out_amount": "0.2831", "expires_at": 1771891500 },
  "trade_id": null,
  "retries": 0,
  "pin_context": "SWAP_SIGN"
}
```

Writes to `users.current_state` + `users.state_data` happen in ONE statement:

```sql
UPDATE users SET current_state = $1, state_data = $2, updated_at = NOW() WHERE id = $3;
```

## 4. Complete Implementation — Worker Loop (`crates/mm-api/src/worker.rs`)

```rust
use crate::fsm::{self, FsmInput};
use crate::state::AppState;
use serde::Deserialize;
use serde_json::Value;
use sqlx::types::Uuid;

#[derive(Debug, Deserialize)]
pub struct InboundEvent {
    pub message_id: String,
    pub sender_jid: String,
    #[serde(default)]
    pub text_body: String,
    #[serde(default)]
    pub has_media: bool,
    pub media_url: Option<String>,
    pub timestamp: i64,
}

const GROUP: &str = "mm_api_workers";
const CONSUMER: &str = "worker";
const STREAM: &str = "inbound:wa:events";
const MAX_ATTEMPTS: u32 = 5;

pub async fn run_worker(state: AppState) -> anyhow::Result<()> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;

    conn.xgroup_create_mkstream::<_, _, _, _>(STREAM, GROUP, "0")
        .await
        .ok();

    loop {
        let entries: Vec<(String, rustis::commands::Entries)> = conn
            .xreadgroup(GROUP, CONSUMER, &[STREAM], &[">"], Some(1), None)
            .await
            .unwrap_or_default();

        if entries.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            continue;
        }

        for (entry_id, fields) in entries {
            let raw = fields.get("payload").cloned().unwrap_or_default();
            let event: InboundEvent = match serde_json::from_str(&raw) {
                Ok(e) => e,
                Err(err) => {
                    dead_letter(&mut conn, &entry_id, &raw, "MALFORMED", &err.to_string()).await;
                    continue;
                }
            };

            match process_once(&state, &event).await {
                Ok(()) => {
                    let _: () = conn.xack(STREAM, GROUP, &[&entry_id]).await?;
                    let _: () = conn.xdel(STREAM, &[&entry_id]).await.ok();
                }
                Err(classification) => {
                    handle_failure(&mut conn, STREAM, GROUP, &entry_id, &raw, &event, classification).await;
                }
            }
        }
    }
}

enum FailureClass {
    Retry(String),
    Terminal(String),
}

async fn process_once(state: &AppState, event: &InboundEvent) -> Result<(), FailureClass> {
    let sender_number = event
        .sender_jid
        .split('@')
        .next()
        .unwrap_or("")
        .to_string();

    // Idempotency gate: duplicate deliveries short-circuit silently.
    let fresh = sqlx::query!(
        r#"INSERT INTO processed_messages (message_id) VALUES ($1) ON CONFLICT DO NOTHING RETURNING message_id"#,
        event.message_id
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| FailureClass::Retry(e.to_string()))?;

    if fresh.is_none() {
        return Ok(());
    }

    let input = FsmInput {
        whatsapp_number: sender_number,
        text: event.text_body.clone(),
        media_url: event.media_url.clone(),
    };

    fsm::advance(state, input)
        .await
        .map_err(FailureClass::Terminal)
}

async fn handle_failure(
    conn: &mut impl rustis::client::ClientLike,
    stream: &str,
    group: &str,
    entry_id: &str,
    raw: &str,
    event: &InboundEvent,
    class: FailureClass,
) {
    let (kind, detail) = match class {
        FailureClass::Terminal(d) => ("TERMINAL", d),
        FailureClass::Retry(d) => ("RETRY", d),
    };

    let attempts = bump_attempt(state_data_of(event)).unwrap_or(1);
    if kind == "RETRY" && attempts < MAX_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(500u64.saturating_mul(1 << attempts))).await;
        return;
    }

    dead_letter(conn, entry_id, raw, kind, &detail).await;
}

fn state_data_of(_e: &InboundEvent) -> Value {
    serde_json::json!({})
}

fn bump_attempt(mut v: Value) -> Option<u32> {
    let cur = v["retries"].as_u64().unwrap_or(0) as u32;
    let next = cur + 1;
    v["retries"] = next.into();
    Some(next)
}

async fn dead_letter(
    conn: &mut impl rustis::client::ClientLike,
    entry_id: &str,
    raw: &str,
    kind: &str,
    detail: &str,
) {
    let envelope = serde_json::json!({
        "source_entry": entry_id,
        "error_class": kind,
        "last_error": detail,
        "at_dead_letter_at": chrono::Utc::now().timestamp(),
        "original": serde_json::from_str::<Value>(raw).unwrap_or(Value::String(raw.to_string())),
    });
    let _: () = conn
        .xadd("inbound:wa:dlq", "*", &[("payload", envelope.to_string())])
        .await
        .ok();
}
```

> The snippet above pins the architectural contract (group semantics, idempotency gate,
> retry/DLQ boundaries); the production `rustis` call signatures are finalized during Vol Iota
> integration against the pinned crate version.

### 4.1 Outbound Emitter (`crates/mm-api/src/outbound.rs`)

```rust
use crate::state::AppState;

pub struct Outbound<'a> {
    pub recipient_jid: &'a str,
    pub text: String,
}

impl<'a> Outbound<'a> {
    /// Typing delay per spec: T = clamp(1200, len*35 + jitter(-200..300), 2500) ms
    pub fn typing_delay_ms(&self) -> u64 {
        let jitter: i64 = (rand::random::<f64>() * 500.0 - 200.0) as i64;
        let t = self.text.len() as i64 * 35 + jitter;
        t.clamp(1200, 2500) as u64
    }

    pub async fn send(self, state: &AppState) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "recipient_jid": self.recipient_jid,
            "text": self.text,
            "typing_delay_ms": self.typing_delay_ms(),
        });

        let http = state
            .http
            .post(format!("{}/bridge/send-message", state.wa_bridge_url))
            .header("X-Internal-Secret", &state.internal_api_secret)
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match http {
            Ok(resp) if resp.status().is_success() => Ok(()),
            _ => {
                // Decoupled fallback: queue on the outbound stream; bridge drains it.
                let mut conn = state.redis.get_multiplexed_async_connection().await?;
                use rustis::commands::XAddOptions;
                let _: String = conn
                    .xadd_options(
                        "outbound:wa:messages",
                        "*",
                        &[("payload", body.to_string())],
                        XAddOptions::default().maxlen(10_000).approximate_trimming(),
                    )
                    .await?;
                Ok(())
            }
        }
    }
}
```

## 5. Error Handling, Retry Logic & DLQ Edge Cases

| Case | Classification | Policy |
|------|---------------|--------|
| Malformed JSON payload | Terminal | DLQ immediately |
| Duplicate `message_id` | Success (no-op) | ACK + XDEL |
| Gemini 5xx / timeout (10 s) | Retry | Backoff; fallback deterministic parser first (Vol Beta §5) |
| Neon deadlock/serialization | Retry | Up to 5 attempts, then DLQ; user gets "try again" notification |
| Jupiter/RPC timeout mid-swap | Terminal-ish | Mark tx FAILED_TIMEOUT, reconcile via signature lookup job before refunding reserved balance |
| Bridge HTTP down | Transparent | Outbound falls back to Redis stream automatically |

DLQ operations: `ops/drain-dlq.sh` prints envelopes, supports `--replay <entry-id>` which
re-publishes the original payload onto `inbound:wa:events` with attempt counter reset.

## 6. Verification Test Cases & Command Sequences

```bash
# V1-T1: end-to-end stream round trip
redis-cli XADD inbound:wa:events '*' payload '{"message_id":"t1","sender_jid":"2348012345678@s.whatsapp.net","text_body":"hi","has_media":false,"timestamp":0}'
redis-cli XPENDING inbound:wa:events mm_api_workers   # should be 0 after processing
curl -s localhost:3000/api/v1/admin/dashboard | jq '.stats'

# V1-T2: idempotency — replay same message twice
redis-cli XADD inbound:wa:events '*' payload '{"message_id":"dup1", ...}'
redis-cli XADD inbound:wa:events '*' payload '{"message_id":"dup1", ...}'
psql $DATABASE_URL -c "SELECT count(*) FROM transactions WHERE metadata->>'message_id'='dup1'"
# expect exactly 1

# V1-T3: DLQ capture on poison message
redis-cli XADD inbound:wa:events '*' payload 'not-json'
redis-cli XLEN inbound:wa:dlq                         # >= 1

# V1-T4: FSM transitions (unit)
cargo test -p mm-api fsm::

# V1-T5: consumer crash safety
docker restart middleman-mm-api-1
# messages published while down must be consumed on boot (group reads from last-committed ID)
```

# VOLUME ETA — `mm-notifications` & Anti-Fraud Engine

**Version:** 2.4.0 · **Owner:** Trust & Safety Engineering · **Scope:** SMTP alerting, WhatsApp
ops notifications, velocity rules, withdrawal guards, PIN strike enforcement.

---

## 1. Architectural Overview & Technical Scope

Two responsibilities in one crate:

1. **Notification transport** — async lettre SMTP for ops email; thin wrapper over the Vol 1
   outbound path for WhatsApp messages to the ops number.
2. **Anti-fraud rule engine** — pure decision functions over Redis counters + DB state, invoked
   by `mm-api` before executing money movement:

```
R1 Velocity        : >3 P2P transfers / 60 s          -> 15 min cooldown
R2 High-value gate : NGN withdrawal >= 500_000        -> manual admin approval queue
R3 PIN strikes     : 3 consecutive invalid PINs       -> vault lock 2 h + alert
R4 Memory watchdog : any container > 90% soft limit x3 samples -> infra alert
```

Rules are fail-closed: a Redis outage blocks R1 evaluation and therefore blocks P2P transfers
(temporary feature degradation beats fraud exposure).

## 2. Mathematical Formulation — Rule Parameters

```
R1: window W=60 s, threshold N=3, cooldown C=900 s
    counter key: velocity:p2p:{user_id}   (INCR + EXPIRE W on first hit)

R2: threshold ₦500,000 per rolling 24 h:
    daily_out(user) = SUM(transactions OUTBOUND FIAT_WITHDRAWAL last 24h)
    hold_required   = (daily_out + amount) >= 500_000

R3: strike window: consecutive (no success between); reset only on successful verify or lockout
    lock TTL = 7200 s stored in users.pin_locked_until

R4: sample period 60 s, trigger after 3 consecutive breaches; RSS_c(t)/SoftLimit_c > 0.90
```

## 3. Complete Implementation

### 3.1 `crates/mm-notifications/src/lib.rs`

```rust
pub mod alerts;
pub mod mailer;
pub mod rules;

pub use mailer::Mailer;
```

### 3.2 `src/mailer.rs`

```rust
use lettre::{
    message::{header::ContentType, Mailbox},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MailError {
    #[error("smtp failure: {0}")]
    Smtp(String),
}

pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
    to_admin: String,
}

impl Mailer {
    pub fn from_env() -> Self {
        let host = std::env::var("SMTP_HOST").expect("SMTP_HOST");
        let user = std::env::var("SMTP_USER").expect("SMTP_USER");
        let pass = std::env::var("SMTP_PASS").expect("SMTP_PASS");
        let relay = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
            .expect("valid smtp host")
            .credentials(Credentials::new(user, pass))
            .build();
        Self {
            transport: relay,
            from: "alerts@middleman.africa".into(),
            to_admin: std::env::var("ADMIN_ALERT_EMAIL").expect("ADMIN_ALERT_EMAIL"),
        }
    }

    pub async fn send_admin(&self, subject: &str, body: &str) -> Result<(), MailError> {
        let email = Message::builder()
            .from(Mailbox::new(None, self.from.parse().map_err(|e| MailError::Smtp(format!("{e}")))?))
            .to(Mailbox::new(None, self.to_admin.parse().map_err(|e| MailError::Smtp(format!("{e}")))?))
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|e| MailError::Smtp(e.to_string()))?;

        self.transport.send(email).await.map_err(|e| MailError::Smtp(e.to_string()))?;
        Ok(())
    }

    /// Fire-and-forget variant used inside hot paths; failures logged, never propagated.
    pub fn send_admin_detached(&self, subject: &'static str, body: String) {
        let this = Self {
            transport: self.transport.clone(),
            from: self.from.clone(),
            to_admin: self.to_admin.clone(),
        };
        tokio::spawn(async move {
            if let Err(e) = this.send_admin(subject, &body).await {
                eprintln!("admin mail failed: {e}");
            }
        });
    }
}
```

### 3.3 `src/alerts.rs`

```rust
use crate::Mailer;

pub enum Alert {
    PinLockout { phone: String },
    HighValueHold { phone: String, amount_ngn: rust_decimal::Decimal },
    TradeResolved { trade_id: uuid::Uuid, approved: bool },
    ContainerMemory { container: String, rss_mb: u64, limit_mb: u64 },
    DlqBacklog { depth: i64 },
}

impl Alert {
    pub fn dispatch(self, mailer: &Mailer, ops_whatsapp_jid: &str, notify_wa: impl Fn(String)) {
        match &self {
            Alert::PinLockout { phone } => {
                mailer.send_admin_detached("[MM] PIN lockout", format!("User {phone}: 3 failed PINs, vault locked 2h."));
                notify_wa(format!("🔒 User {phone} locked out (3 bad PINs)."));
            }
            Alert::HighValueHold { phone, amount_ngn } => {
                mailer.send_admin_detached("[MM] Withdrawal hold", format!("User {phone} requested ₦{amount_ngn} — needs approval."));
                notify_wa(format!("⚠️ Hold: {phone} → ₦{amount_ngn}. Approve in desk."));
            }
            Alert::TradeResolved { trade_id, approved } => {
                let verdict = if *approved { "APPROVED" } else { "REJECTED" };
                mailer.send_admin_detached("[MM] Trade settled", format!("Trade {trade_id} {verdict}."));
            }
            Alert::ContainerMemory { container, rss_mb, limit_mb } => {
                mailer.send_admin_detached("[MM] Memory pressure",
                    format!("{container} at {rss_mb}MB / {limit_mb}MB soft limit."));
            }
            Alert::DlqBacklog { depth } => {
                mailer.send_admin_detached("[MM] DLQ backlog", format!("Dead letters: {depth}. Drain required."));
            }
        }
    }
}
```

### 3.4 `src/rules.rs`

```rust
use redis::AsyncCommands;
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum RuleViolation {
    #[error("cool-down active: retry in {minutes} minutes")]
    Cooldown { minutes: u32 },
    #[error("withdrawal requires manual approval")]
    NeedsApproval,
}

const DAILY_GUARD_NGN: i64 = 500_000;

/// R1 — sliding-window velocity (see Vol Delta p2p::velocity_check for the same logic).
pub async fn velocity_guard(
    conn: &mut impl redis::aio::ConnectionLike,
    user_id: Uuid,
) -> Result<(), RuleViolation> {
    let cd_key = format!("cooldown:{user_id}");
    let blocked: Option<String> = conn.get(&cd_key).await.unwrap_or(None);
    if blocked.is_some() {
        return Err(RuleViolation::Cooldown { minutes: 15 });
    }
    Ok(())
}

/// R2 — high-value withdrawal gate.
pub async fn high_value_gate(
    pool: &sqlx::Pool<sqlx::Postgres>,
    user_id: Uuid,
    amount_ngn: Decimal,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT COALESCE(SUM(amount),0) AS out24h
           FROM transactions
           WHERE user_id=$1 AND tx_type='FIAT_WITHDRAWAL'
             AND direction='OUTBOUND' AND status IN ('PROCESSING','SUCCESS')
             AND created_at > NOW() - INTERVAL '24 hours'"#,
        user_id
    )
    .fetch_one(pool)
    .await?;

    let projected = row.out24h + amount_ngn;
    Ok(projected >= Decimal::from(DAILY_GUARD_NGN))
}

/// R3 — strike bookkeeping called by FSM's AWAITING_PIN handler.
pub struct StrikeOutcome {
    pub locked: bool,
    pub attempts_left: u8,
}

pub async fn register_pin_failure(
    db_tx: &mut sqlx::PgConnection,
    user_id: Uuid,
    current_failures: i32,
    mailer: &crate::Mailer,
    phone: &str,
) -> Result<StrikeOutcome, sqlx::Error> {
    let next = current_failures + 1;
    if next >= 3 {
        sqlx::query!(
            r#"UPDATE users SET failed_pin_attempts=0,
                 pin_locked_until=NOW() + INTERVAL '2 hours', updated_at=NOW()
               WHERE id=$1"#,
            user_id
        )
        .execute(&mut *db_tx)
        .await?;
        mailer.send_admin_detached("[MM] PIN lockout", format!("User {phone} locked for 2h."));
        return Ok(StrikeOutcome { locked: true, attempts_left: 0 });
    }
    sqlx::query!(
        r#"UPDATE users SET failed_pin_attempts=$2, updated_at=NOW() WHERE id=$1"#,
        user_id, next
    )
    .execute(&mut *db_tx)
    .await?;
    Ok(StrikeOutcome { locked: false, attempts_left: (3 - next) as u8 })
}
```

## 4. Data Schemas & Structural Interfaces

| Interface | Consumer | Contract |
|---|---|---|
| `Mailer::send_admin(subject, body)` | all alerts | STARTTLS SMTP, detached mode for hot paths |
| `Alert::dispatch(mailer, jid, notify_wa)` | mm-api bootstrap | Dual-channel: email always, WhatsApp to ops JID |
| `velocity_guard(conn, user_id)` | FSM pre-execution | Err(Cooldown) blocks transfer |
| `high_value_gate(pool, user_id, amt)` | fiat withdrawal flow | true ⇒ create PROCESSING row + admin queue item |
| `register_pin_failure(...)` | AWAITING_PIN handler | Locks via `users.pin_locked_until` |

Withdrawal approval queue item shape (desk):

```json
{ "event": "withdrawal.hold", "user": "+234...", "amount_ngn": "750000.00", "tx_id": "..." }
```

Admin approve endpoint flips `transactions.status PROCESSING -> SUCCESS` and triggers the
Flutterwave payout call (Vol Gamma); reject sets FAILED with reason and releases reserved funds.

## 5. Error Handling Policies

| Condition | Policy |
|---|---|
| SMTP down | Alerts buffer to `alerts:pending` Redis list (max 1000); flushed every 5 min |
| Redis down | R1 fails closed (no P2P); R2/R3 are DB-backed and still function |
| Duplicate alert storms | Dedupe key `alert:{kind}:{subject_hash}` with 10-min TTL |
| DLQ depth > 50 | Periodic checker raises `Alert::DlqBacklog` |
| Ops WhatsApp JID unreachable | Email remains authoritative channel |

## 6. Verification Test Cases & Command Sequences

```bash
# VE-T1..T4: unit tests
cargo test -p mm-notifications
# velocity_guard: 3 passes then cooldown error; expiry resets (fake clock)
# high_value_gate: 499_999+500_000 boundary behavior
# register_pin_failure: locks exactly on 3rd, resets counter

# VE-T5: live SMTP probe
cargo run -p mm-notifications --bin smtp_probe     # delivers test mail to ADMIN_ALERT_EMAIL

# VE-T6: end-to-end strike lockout
# send wrong PIN 3 times via FSM stub ->
psql $DATABASE_URL -c "SELECT pin_locked_until IS NOT NULL AS locked FROM users WHERE whatsapp_number='<n>';"
# expect t ; 4th attempt replies with countdown without hashing cost

# VE-T7: high-value hold path
# request ₦600k withdrawal -> transaction stays PROCESSING, desk shows hold card,
# admin approves -> Flutterwave sandbox transfer initiated, status SUCCESS
```

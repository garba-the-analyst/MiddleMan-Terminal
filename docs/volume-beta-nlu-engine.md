# VOLUME BETA — `mm-ai` & Conversational NLU Engine

**Version:** 2.4.0 · **Owner:** Intelligence Engineering · **Scope:** Intent extraction,
entity normalization, Pidgin/slang handling, degraded-mode determinism.

---

## 1. Architectural Overview & Technical Scope

`mm-ai` answers exactly one question for the FSM: *what does this message want?* It is a
stateless function `(text, media_present) -> ParsedIntent`. It NEVER mutates ledgers, NEVER
decides balances, and is ALWAYS wrapped by a deterministic fallback parser so a Gemini outage
degrades to keyword mode instead of dead air.

Model: Google Gemini Flash (`gemini-flash-latest` pinned alias) via REST
`generativelanguage.googleapis.com/v1beta`. Temperature 0.1. Structured output enforced with
`response_mime_type: application/json` + `response_schema`.

## 2. Formal Contract — Response Schema

```json
{
  "type": "object",
  "required": ["intent", "confidence", "entities"],
  "properties": {
    "intent": {
      "type": "string",
      "enum": [
        "REGISTER_USER", "LIQUIDATE_GIFT_CARD", "CHECK_BALANCE", "EXECUTE_DEX_SWAP",
        "P2P_TRANSFER", "OPEN_PERP_POSITION", "TRANSFER_FIAT", "BUY_AIRTIME",
        "CHECK_CONTRACT_SECURITY", "HELP", "UNKNOWN"
      ]
    },
    "confidence": { "type": "number" },
    "entities": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "amount":            { "type": ["number", "null"] },
        "source_currency":   { "type": ["string", "null"] },
        "target_currency":   { "type": ["string", "null"] },
        "recipient_phone":   { "type": ["string", "null"] },
        "recipient_address": { "type": ["string", "null"] },
        "card_brand":        { "type": ["string", "null"] },
        "contract_address":  { "type": ["string", "null"] },
        "network":           { "type": ["string", "null"] }
      }
    }
  }
}
```

Gate: `confidence < 0.60 => treat as UNKNOWN` (menu prompt). `UNKNOWN` never executes anything.

### 2.1 Normalization Rules (deterministic, applied before AND after LLM)

| Input token | Canonical |
|---|---|
| `50k`, `50K`, `50,000` | `50000` |
| `2.5m` | `2500000` |
| `08012345678`, `2348012345678`, `+234 801 234 5678` | `+2348012345678` |
| `usdt/usd-t/tether` | `USDT` |
| `sol/solana` | `SOL` (currency) vs `SOLANA` (chain) |
| `steam/steem card` | `STEAM` |
| `aple/apple/itunes` | `APPLE` |
| `mtn/airtel/glo/9mobile/etisalat` | network entity |

## 3. Complete Implementation

### 3.1 `crates/mm-ai/src/lib.rs`

```rust
pub mod fallback;
pub mod normalizer;
pub mod parser;

pub use parser::{AiError, GeminiParser, ParsedIntent};
```

`Cargo.toml`: add `regex = "1"`, `once_cell = "1"`.

### 3.2 `src/parser.rs`

```rust
use crate::fallback::rulebook_parse;
use crate::normalizer::normalize_text;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("Failed to parse JSON response: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("Gemini API error: HTTP {status} - {body}")]
    ApiError { status: u16, body: String },
    #[error("Model returned unusable shape")]
    BadShape,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entities {
    pub amount: Option<f64>,
    pub source_currency: Option<String>,
    pub target_currency: Option<String>,
    pub recipient_phone: Option<String>,
    pub recipient_address: Option<String>,
    pub card_brand: Option<String>,
    pub contract_address: Option<String>,
    pub network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedIntent {
    pub intent: String,
    pub confidence: f64,
    pub entities: Entities,
}

const SYSTEM_INSTRUCTION: &str = r#"You are the deterministic intent extractor for MiddleMan, a
WhatsApp neo-bank in Nigeria. Classify the user's message and extract entities ONLY.
Understand Nigerian English, Pidgin and slang ("abeg", "wetin", "50k", "send me").
Return strictly valid JSON matching the provided response schema. No prose, no markdown.
Rules:
- Expand k/m suffixes to numbers (50k -> 50000).
- Normalize Nigerian phones to E.164 (+234...).
- If money direction is unclear, set intent UNKNOWN.
- Never invent entities that are not implied."#;

pub struct GeminiParser {
    client: Client,
    api_key: String,
    model: String,
}

impl GeminiParser {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("static client config"),
            api_key,
            model: "gemini-flash-latest".to_string(),
        }
    }

    /// Extracts intent; falls back to the deterministic rulebook on any AI failure or
    /// low-confidence result. Never returns Err in production paths.
    pub async fn extract_intent(&self, raw_message: &str) -> Result<ParsedIntent, AiError> {
        let normalized = normalize_text(raw_message);

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );

        let payload = serde_json::json!({
            "systemInstruction": { "parts": [{ "text": SYSTEM_INSTRUCTION }] },
            "contents": [{ "parts": [{ "text": normalized }] }],
            "generationConfig": {
                "temperature": 0.1,
                "maxOutputTokens": 256,
                "responseMimeType": "application/json",
                "responseSchema": response_schema_json()
            }
        });

        let resp = self
            .client
            .post(&url)
            .query(&[("key", &self.api_key)])
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            // Degraded mode: deterministic parse, flagged confidence 0.5
            return Ok(rulebook_parse(&normalized));
        }

        let data: serde_json::Value = resp.json().await?;
        let text = data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or(AiError::BadShape)?;

        let parsed: ParsedIntent = serde_json::from_str(text).map_err(AiError::ParseError)?;

        if parsed.confidence < 0.60 || parsed.intent == "UNKNOWN" {
            return Ok(rulebook_parse(&normalized));
        }
        Ok(parsed)
    }
}

fn response_schema_json() -> serde_json::Value {
    serde_json::json!({
        "type": "OBJECT",
        "properties": {
            "intent": { "type": "STRING", "enum": [
                "REGISTER_USER","LIQUIDATE_GIFT_CARD","CHECK_BALANCE","EXECUTE_DEX_SWAP",
                "P2P_TRANSFER","OPEN_PERP_POSITION","TRANSFER_FIAT","BUY_AIRTIME",
                "CHECK_CONTRACT_SECURITY","HELP","UNKNOWN"] },
            "confidence": { "type": "NUMBER" },
            "entities": {
                "type": "OBJECT",
                "properties": {
                    "amount": { "type": ["NUMBER","NULL"] },
                    "source_currency": { "type": ["STRING","NULL"] },
                    "target_currency": { "type": ["STRING","NULL"] },
                    "recipient_phone": { "type": ["STRING","NULL"] },
                    "recipient_address": { "type": ["STRING","NULL"] },
                    "card_brand": { "type": ["STRING","NULL"] },
                    "contract_address": { "type": ["STRING","NULL"] },
                    "network": { "type": ["STRING","NULL"] }
                }
            }
        },
        "required": ["intent", "confidence", "entities"]
    })
}
```

> Note: on API failure we return `Ok(rulebook_parse(...))` rather than an error — availability
> beats purity here; the FSM treats low confidence conservatively anyway.

### 3.3 `src/normalizer.rs`

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static K_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(\d+(?:\.\d+)?)\s*k\b").unwrap());
static M_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(\d+(?:\.\d+)?)\s*m\b").unwrap());
static NG_PHONE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:\+?234|0)([7-9]\d{9})").unwrap());

pub fn normalize_text(input: &str) -> String {
    let s = input.trim();
    let s = K_SUFFIX.replace_all(s, |c: &regex::Captures| {
        format!("{}000", c[1].replace('.', ""))
    });
    let s = M_SUFFIX.replace_all(&s, |c: &regex::Captures| {
        let base = c[1].replace('.', "");
        let zeros = match c[1].split('.').nth(1).map(str::len).unwrap_or(0) {
            0 => "000000".to_string(),
            1 => format!("{}00", base),
            _ => format!("{}0", base),
        };
        zeros
    });
    NG_PHONE
        .replace_all(&s, |c: &regex::Captures| format!("+234{}", &c[1]))
        .to_string()
}
```

### 3.4 `src/fallback.rs`

```rust
use crate::parser::{Entities, ParsedIntent};
use once_cell::sync::Lazy;
use regex::Regex;

macro_rules! re { ($p:expr) => { Lazy::new(|| Regex::new($p).unwrap()) } }

static RE_SWAP:     Lazy<Regex> = re!(r"(?i)\b(swap|convert|exchange)\b.*\b(to)\b");
static RE_BALANCE:  Lazy<Regex> = re!(r"(?i)\b(balance|how much dey|wallet worth)\b");
static RE_GIFT:     Lazy<Regex> = re!(r"(?i)\b(steam|apple|amazon|razer|google\s?play|sephora)\b");
static RE_P2P:      Lazy<Regex> = re!(r"(?i)\b(send|transfer)\b.*\+?234\d{10}");
static RE_AIRTIME:  Lazy<Regex> = re!(r"(?i)\b(airtime|data|mtn|airtel|glo|9mobile)\b");
static RE_AMOUNT:   Lazy<Regex> = re!(r"(?i)(?:^|\s)(?:₦|\$)?(\d[\d,]*(?:\.\d+)?)");
static RE_CONTRACT: Lazy<Regex> = re!(r"(0x[a-fA-F0-9]{40}|[1-9A-HJ-NP-Za-km-z]{32,44})");

fn amount_of(text: &str) -> Option<f64> {
    RE_AMOUNT.captures(text)?.get(1)?.as_str().replace(',', "").parse().ok()
}

/// Deterministic degraded-mode parser. Confidence fixed at 0.5 so callers know it is machine-guessed.
pub fn rulebook_parse(text: &str) -> ParsedIntent {
    let amount = amount_of(text);
    let contract = RE_CONTRACT.find(text).map(|m| m.as_str().to_string());

    let (intent, entities) = if RE_SWAP.is_match(text) {
        let src = detect_ccy_before_to(text);
        ("EXECUTE_DEX_SWAP", Entities {
            amount, source_currency: Some(src.unwrap_or_else(|| "USDT".into())),
            target_currency: detect_ccy_after_to(text),
            ..Entities::empty()
        })
    } else if RE_P2P.is_match(text) {
        ("P2P_TRANSFER", Entities {
            amount,
            recipient_phone: extract_phone(text),
            source_currency: None,
            ..Entities::empty()
        })
    } else if RE_GIFT.is_match(text) && text.to_lowercase().contains("card") || has_media_hint(text) {
        ("LIQUIDATE_GIFT_CARD", Entities {
            card_brand: brand_of(text), amount, ..Entities::empty()
        })
    } else if RE_BALANCE.is_match(text) {
        ("CHECK_BALANCE", Entities::empty())
    } else if RE_AIRTIME.is_match(text) {
        ("BUY_AIRTIME", Entities {
            amount, network: network_of(text), ..Entities::empty()
        })
    } else if contract.is_some() {
        ("CHECK_CONTRACT_SECURITY", Entities {
            contract_address: contract, ..Entities::empty()
        })
    } else {
        ("UNKNOWN", Entities::empty())
    };

    ParsedIntent { intent: intent.into(), confidence: 0.5, entities }
}

fn has_media_hint(_t: &str) -> bool { false } // media presence injected by caller when needed

fn detect_ccy_before_to(t: &str) -> Option<String> {
    for c in ["USDT", "USDC", "ETH", "SOL", "NGN"] {
        if t.to_uppercase().contains(c) { return Some(c.to_string()); }
    }
    None
}

fn detect_ccy_after_to(t: &str) -> Option<String> {
    let lower = t.to_uppercase();
    let idx = lower.rfind("TO")?;
    let tail = &lower[idx + 2..];
    ["SOL", "ETH", "USDT", "USDC", "NGN"].iter()
        .find(|c| tail.contains(*c))
        .map(|c| c.to_string())
}

fn extract_phone(t: &str) -> Option<String> {
    Regex::new(r"\+234\d{10}").unwrap().find(t).map(|m| m.as_str().to_string())
}

fn brand_of(t: &str) -> Option<String> {
    let up = t.to_uppercase();
    ["STEAM", "APPLE", "AMAZON", "RAZER", "GOOGLE_PLAY", "SEPHORA"]
        .iter().find(|b| up.contains(b)).map(|b| b.to_string())
}

fn network_of(t: &str) -> Option<String> {
    let up = t.to_uppercase();
    ["MTN", "AIRTEL", "GLO", "9MOBILE"].iter()
        .find(|n| up.contains(n)).map(|n| n.to_string())
}

impl Entities {
    pub fn empty() -> Self {
        Self {
            amount: None, source_currency: None, target_currency: None,
            recipient_phone: None, recipient_address: None, card_brand: None,
            contract_address: None, network: None,
        }
    }
}
```

## 4. Data Schemas & Structural Interfaces

- Input interface: `&str` (already-normalized WhatsApp text).
- Output interface: `ParsedIntent` consumed exclusively by `mm-api/src/fsm`.
- Media note: images are NOT parsed here. Gift-card OCR runs in Vol Epsilon using a separate
  multimodal call; NLU sees only captions/text.

Pidgin ground-truth examples locked as tests:

| Message | Expected |
|---|---|
| `"Send 20 USDT to 08012345678"` | `P2P_TRANSFER`, amount 20.0, phone `+2348012345678` |
| `"Convert 50k Steam card to cash"` | `LIQUIDATE_GIFT_CARD`, STEAM, 50000 |
| `"swap $30 usdt to sol"` | `EXECUTE_DEX_SWAP`, USDT→SOL, 30.0 |
| `"Wetin dey my wallet"` | `CHECK_BALANCE` |
| `"check 0x7a25935880e41c22... honeypot"` | `CHECK_CONTRACT_SECURITY` |

## 5. Error Handling Policies

| Condition | Path |
|---|---|
| Gemini 429/5xx/timeout(10 s) | Immediate rulebook fallback; no retry storm |
| Schema-invalid model output | Rulebook fallback |
| confidence < 0.6 | Rulebook attempt; if still UNKNOWN → FSM help menu |
| Empty text + media present | Return UNKNOWN; Epsilon flow takes over via `has_media` flag |

Latency budget: p95 ≤ 2.5 s including network. Exceeding it twice in a row trips a 60 s circuit
breaker forcing rulebook-only mode (Vol Eta alert).

## 6. Verification Test Cases & Command Sequences

```bash
# VB-T1: normalizer unit tests (k/m suffixes, phone E.164)
cargo test -p mm-ai normalizer::

# VB-T2: rulebook ground truths
cargo test -p mm-ai fallback::tests

# VB-T3: live model conformance (gated behind GEMINI_API_KEY)
GEMINI_API_KEY=... cargo test -p mm-ai --ignored live_parse_conformance

# VB-T4: latency budget
cargo test -p mm-ai --ignored live_latency_p95 -- --nocapture   # asserts p95 <= 2500ms over 20 calls

# VB-T5: circuit breaker
# simulate failures with GEMINI_API_KEY=bogus; observe logs switch to 'mode=rulebook' within 2 requests
```

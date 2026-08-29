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
    #[error("Gemini API error: HTTP {status}")]
    ApiError { status: u16 },
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
            model: std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-latest".into()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// Extracts intent; falls back to the deterministic rulebook on any AI failure or
    /// low-confidence result. Production paths receive Ok(ParsedIntent) either way.
    pub async fn extract_intent(&self, raw_message: &str) -> Result<ParsedIntent, AiError> {
        let normalized = normalize_text(raw_message);

        if !self.is_enabled() {
            return Ok(rulebook_parse(&normalized));
        }

        match self.call_gemini(&normalized).await {
            Ok(parsed) if parsed.confidence >= 0.60 && parsed.intent != "UNKNOWN" => Ok(parsed),
            _ => Ok(rulebook_parse(&normalized)),
        }
    }

    async fn call_gemini(&self, normalized: &str) -> Result<ParsedIntent, AiError> {
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
            return Err(AiError::ApiError { status: resp.status().as_u16() });
        }

        let data: serde_json::Value = resp.json().await?;
        let text = data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or(AiError::BadShape)?;

        serde_json::from_str(text).map_err(AiError::ParseError)
    }
}

fn response_schema_json() -> serde_json::Value {
    serde_json::json!({
        "type": "OBJECT",
        "properties": {
            "intent": { "type": "STRING", "enum": [
                "REGISTER_USER","LIQUIDATE_GIFT_CARD","CHECK_BALANCE","EXECUTE_DEX_SWAP",
                "P2P_TRANSFER","OPEN_PERP_POSITION","TRANSFER_FIAT","BUY_AIRTIME",
                "CHECK_CONTRACT_SECURITY","HELP","UNKNOWN","SET_PIN","CREATE_FOREIGN_ACCOUNT"] },
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

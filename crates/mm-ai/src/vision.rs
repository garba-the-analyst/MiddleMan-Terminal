use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VisionError {
    #[error("image fetch failed: {0}")]
    Fetch(String),
    #[error("gemini request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("gemini returned HTTP {status}")]
    Api { status: u16 },
    #[error("model returned unusable output")]
    BadShape,
    #[error("vision disabled: no API key")]
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardRead {
    pub brand: String,
    pub country: String,
    #[serde(rename = "format")]
    pub card_format: String,
    pub usd_value: f64,
    pub code: String,
}

impl CardRead {
    pub fn is_usable(&self) -> bool {
        self.brand != "OTHER" && self.usd_value >= 1.0
    }
}

const VISION_PROMPT: &str = r#"You are an OCR engine for a Nigerian gift-card exchange.
Read this gift card image and return STRICT JSON only:
{"brand":"STEAM|APPLE|AMAZON|RAZER_GOLD|GOOGLE_PLAY|OTHER",
 "country":"US|UK|DE|CA|OTHER",
 "format":"PHYSICAL|ECODE",
 "usd_value": number,
 "code":"the alphanumeric redemption code exactly as printed, or empty string"}
If the image is unreadable or not a gift card, return
{"brand":"OTHER","country":"OTHER","format":"PHYSICAL","usd_value":0,"code":""}.
No prose, no markdown fences."#;

fn response_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "OBJECT",
        "properties": {
            "brand":   { "type": "STRING" },
            "country": { "type": "STRING" },
            "format":  { "type": "STRING" },
            "usd_value": { "type": "NUMBER" },
            "code":    { "type": "STRING" }
        },
        "required": ["brand", "country", "format", "usd_value", "code"]
    })
}

/// Downloads the hosted card image and asks Gemini to extract structured card data.
/// Uses inline_data (base64) so any public URL works — no Files API upload needed.
pub async fn read_card_image(api_key: &str, image_url: &str) -> Result<CardRead, VisionError> {
    if api_key.trim().is_empty() {
        return Err(VisionError::Disabled);
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()?;

    let img = http
        .get(image_url)
        .send()
        .await
        .map_err(|e| VisionError::Fetch(e.to_string()))?;

    if !img.status().is_success() {
        return Err(VisionError::Fetch(format!("HTTP {}", img.status())));
    }

    let mime = img
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .split(';')
        .next()
        .unwrap_or("image/jpeg")
        .to_string();

    let bytes = img
        .bytes()
        .await
        .map_err(|e| VisionError::Fetch(e.to_string()))?;
    let encoded = STANDARD.encode(&bytes);

    let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-latest".into());
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
    );

    let payload = serde_json::json!({
        "systemInstruction": { "parts": [{ "text": VISION_PROMPT }] },
        "contents": [{
            "parts": [
                { "text": "Extract this gift card's data." },
                { "inline_data": { "mime_type": mime, "data": encoded } }
            ]
        }],
        "generationConfig": {
            "temperature": 0.0,
            "maxOutputTokens": 256,
            "responseMimeType": "application/json",
            "responseSchema": response_schema()
        }
    });

    let resp = http
        .post(&url)
        .query(&[("key", api_key)])
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(VisionError::Api { status: resp.status().as_u16() });
    }

    let data: serde_json::Value = resp.json().await?;
    let text = data["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or(VisionError::BadShape)?;

    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    serde_json::from_str::<CardRead>(cleaned).map_err(|_| VisionError::BadShape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usability_gate() {
        let bad = CardRead {
            brand: "OTHER".into(),
            country: "OTHER".into(),
            card_format: "PHYSICAL".into(),
            usd_value: 0.0,
            code: String::new(),
        };
        assert!(!bad.is_usable());

        let good = CardRead {
            brand: "STEAM".into(),
            country: "US".into(),
            card_format: "PHYSICAL".into(),
            usd_value: 50.0,
            code: "ABC123".into(),
        };
        assert!(good.is_usable());
    }

    #[test]
    fn parses_model_json_with_fences() {
        let raw = "```json\n{\"brand\":\"APPLE\",\"country\":\"US\",\"format\":\"ECODE\",\"usd_value\":100,\"code\":\"XJ4\"}\n```";
        let cleaned = raw
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        let parsed: CardRead = serde_json::from_str(cleaned).unwrap();
        assert_eq!(parsed.brand, "APPLE");
        assert_eq!(parsed.usd_value, 100.0);
        assert_eq!(parsed.code, "XJ4");
    }

    #[tokio::test]
    async fn disabled_without_key() {
        let err = read_card_image("", "https://example.com/x.jpg").await;
        assert!(matches!(err, Err(VisionError::Disabled)));
    }
}

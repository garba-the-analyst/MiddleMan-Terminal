use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RadarError {
    #[error("scan request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("token not found on GoPlus for this chain")]
    NotFound,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenSecurity {
    #[serde(default)]
    pub is_honeypot: Option<String>,
    #[serde(default)]
    pub sell_tax: Option<String>,
    #[serde(default)]
    pub buy_tax: Option<String>,
    #[serde(default)]
    pub is_mintable: Option<String>,
    #[serde(default)]
    pub owner_change_balance: Option<String>,
    #[serde(default)]
    pub freezable: Option<String>,
    #[serde(default)]
    pub token_name: Option<String>,
    #[serde(default)]
    pub token_symbol: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Blocked(String),
    Warn(String),
    Clear,
}

fn pct(v: &Option<String>) -> f64 {
    v.as_deref()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| if f <= 1.0 { f * 100.0 } else { f })
        .unwrap_or(0.0)
}

fn flag(v: &Option<String>) -> bool {
    matches!(v.as_deref(), Some("1") | Some("true"))
}

/// Vol Delta §2.3 gate: block honeypots, high tax, live mint authority.
pub fn enforce(t: &TokenSecurity) -> Verdict {
    if flag(&t.is_honeypot) {
        return Verdict::Blocked("honeypot detected — you cannot sell after buying".into());
    }
    let sell = pct(&t.sell_tax);
    if sell > 10.0 {
        return Verdict::Blocked(format!("sell tax {sell:.1}% exceeds 10% limit"));
    }
    let buy = pct(&t.buy_tax);
    if buy > 15.0 {
        return Verdict::Blocked(format!("buy tax {buy:.1}% exceeds 15% limit"));
    }
    if flag(&t.is_mintable) {
        return Verdict::Blocked("mint authority still active — supply can be inflated".into());
    }
    if flag(&t.owner_change_balance) {
        return Verdict::Warn("owner can modify balances — proceed only if you accept the risk".into());
    }
    if flag(&t.freezable) {
        return Verdict::Warn("token can be frozen by its authority".into());
    }
    Verdict::Clear
}

fn detect_evm(address: &str) -> bool {
    address.starts_with("0x") && address.len() == 42
}

/// Nested Solana flags arrive as {"status":"0","authority":[]} — flatten to "0"/"1".
fn nested_status(node: &serde_json::Value, key: &str) -> Option<String> {
    match &node[key] {
        serde_json::Value::Object(map) => map
            .get("status")
            .and_then(|s| s.as_str())
            .map(str::to_string),
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn parse_solana(node: &serde_json::Value) -> TokenSecurity {
    let meta = &node["metadata"];
    TokenSecurity {
        is_honeypot: Some("0".into()),
        sell_tax: node["transfer_fee"]
            .as_object()
            .and_then(|m| m.get("fee_rate").and_then(|v| v.as_str()).map(str::to_string))
            .or_else(|| Some("0".into())),
        buy_tax: Some("0".into()),
        is_mintable: nested_status(node, "mintable"),
        owner_change_balance: nested_status(node, "balance_mutable_authority"),
        freezable: nested_status(node, "freezable"),
        token_name: meta["name"].as_str().map(str::to_string),
        token_symbol: meta["symbol"].as_str().map(str::to_string),
    }
}

fn parse_evm(node: &serde_json::Value) -> TokenSecurity {
    TokenSecurity {
        is_honeypot: node["is_honeypot"].as_str().map(str::to_string),
        sell_tax: node["sell_tax"].as_str().map(str::to_string),
        buy_tax: node["buy_tax"].as_str().map(str::to_string),
        is_mintable: node["is_mintable"].as_str().map(str::to_string),
        owner_change_balance: node["owner_change_balance"].as_str().map(str::to_string),
        freezable: node["cannot_sell_all"].as_str().map(str::to_string),
        token_name: node["token_name"].as_str().map(str::to_string),
        token_symbol: node["token_symbol"].as_str().map(str::to_string),
    }
}

pub async fn scan_token(address: &str) -> Result<TokenSecurity, RadarError> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;

    if detect_evm(address) {
        // Try Ethereum first, then BSC — most Nigerian degen tokens live on one of the two.
        for chain in ["1", "56"] {
            let url = format!(
                "https://api.gopluslabs.io/api/v1/token_security/{chain}?contract_addresses={address}"
            );
            let v: serde_json::Value = http.get(&url).send().await?.json().await?;
            if let Some(node) = first_result(&v) {
                return Ok(parse_evm(&node));
            }
        }
        return Err(RadarError::NotFound);
    }

    let url =
        format!("https://api.gopluslabs.io/api/v1/solana/token_security?contract_addresses={address}");
    let v: serde_json::Value = http.get(&url).send().await?.json().await?;
    let node = first_result(&v).ok_or(RadarError::NotFound)?;
    Ok(parse_solana(&node))
}

fn first_result(v: &serde_json::Value) -> Option<serde_json::Value> {
    let result = v.get("result")?;
    match result {
        serde_json::Value::Object(map) => map.values().next().cloned(),
        serde_json::Value::Array(items) => items.first().cloned(),
        _ => None,
    }
    .filter(|n| !n.is_null() && n.as_object().map(|m| !m.is_empty()).unwrap_or(false))
}

pub fn format_report(address: &str, t: &TokenSecurity, verdict: &Verdict) -> String {
    let name = t
        .token_symbol
        .clone()
        .or_else(|| t.token_name.clone())
        .unwrap_or_else(|| "Unknown token".into());

    let header = match verdict {
        Verdict::Blocked(reason) => format!("🚨 *UNSAFE — DO NOT TRADE*\n\n{reason}"),
        Verdict::Warn(reason) => format!("⚠️ *RISKY*\n\n{reason}"),
        Verdict::Clear => "✅ *Passed safety checks*".to_string(),
    };

    format!(
        "{header}\n\n*Token:* {name}\n*Contract:* {}...{}\n*Honeypot:* {}\n*Buy tax:* {:.1}%\n*Sell tax:* {:.1}%\n*Mintable:* {}",
        &address[..address.len().min(8)],
        &address[address.len().saturating_sub(6)..],
        if flag(&t.is_honeypot) { "YES" } else { "no" },
        pct(&t.buy_tax),
        pct(&t.sell_tax),
        if flag(&t.is_mintable) { "YES" } else { "no" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> TokenSecurity {
        TokenSecurity {
            is_honeypot: Some("0".into()),
            sell_tax: Some("0".into()),
            buy_tax: Some("0".into()),
            is_mintable: Some("0".into()),
            owner_change_balance: Some("0".into()),
            freezable: Some("0".into()),
            token_name: Some("Test".into()),
            token_symbol: Some("TST".into()),
        }
    }

    #[test]
    fn honeypot_is_blocked() {
        let mut t = base();
        t.is_honeypot = Some("1".into());
        assert!(matches!(enforce(&t), Verdict::Blocked(_)));
    }

    #[test]
    fn high_sell_tax_blocked() {
        let mut t = base();
        t.sell_tax = Some("0.125".into()); // 12.5%
        assert!(matches!(enforce(&t), Verdict::Blocked(_)));
    }

    #[test]
    fn mintable_blocked() {
        let mut t = base();
        t.is_mintable = Some("1".into());
        assert!(matches!(enforce(&t), Verdict::Blocked(_)));
    }

    #[test]
    fn owner_change_balance_warns() {
        let mut t = base();
        t.owner_change_balance = Some("1".into());
        assert!(matches!(enforce(&t), Verdict::Warn(_)));
    }

    #[test]
    fn clean_token_clears() {
        assert_eq!(enforce(&base()), Verdict::Clear);
    }

    #[test]
    fn chain_detection() {
        assert!(detect_evm("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"));
        assert!(!detect_evm("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"));
    }

    #[test]
    fn solana_nested_flags_flatten() {
        let node = serde_json::json!({
            "mintable": { "status": "1", "authority": [] },
            "balance_mutable_authority": { "status": "0", "authority": [] },
            "freezable": { "status": "0", "authority": [] },
            "transfer_fee": {},
            "metadata": { "name": "Bonk", "symbol": "Bonk" }
        });
        let parsed = parse_solana(&node);
        assert_eq!(parsed.token_symbol.as_deref(), Some("Bonk"));
        assert!(matches!(enforce(&parsed), Verdict::Blocked(_)));
    }

    #[test]
    fn empty_result_is_not_found() {
        let v = serde_json::json!({ "result": {} });
        assert!(first_result(&v).is_none());
    }

    #[test]
    fn report_renders_verdict() {
        let t = base();
        let r = format_report("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D", &t, &Verdict::Clear);
        assert!(r.contains("Passed safety checks"));
        assert!(r.contains("TST"));
    }
}

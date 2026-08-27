use crate::parser::Entities;
use once_cell::sync::Lazy;
use regex::Regex;

static RE_SWAP: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(swap|convert|exchange)\b").unwrap());
static RE_BALANCE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(balance|how much dey|wallet worth|account worth)\b").unwrap());
static RE_GIFT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(steam|apple|amazon|razer|google\s?play|sephora)\b").unwrap());
static RE_P2P: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(send|transfer)\b.*\+?234\d{10}").unwrap());
static RE_AIRTIME: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(airtime|data|mtn|airtel|glo|9mobile)\b").unwrap());
static RE_AMOUNT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?:^|\s)(?:₦|\$)?(\d[\d,]*(?:\.\d+)?)").unwrap());
static RE_CONTRACT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(0x[a-fA-F0-9]{40}|[1-9A-HJ-NP-Za-km-z]{32,44})").unwrap());
static RE_PHONE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\+234\d{10}").unwrap());

const KNOWN_BRANDS: [&str; 7] = [
    "STEAM",
    "APPLE",
    "AMAZON",
    "RAZER",
    "GOOGLE_PLAY",
    "GOOGLE PLAY",
    "SEPHORA",
];
const KNOWN_NETWORKS: [&str; 4] = ["MTN", "AIRTEL", "GLO", "9MOBILE"];
const KNOWN_CCYS: [&str; 5] = ["USDT", "USDC", "ETH", "SOL", "NGN"];

pub fn empty_entities() -> Entities {
    Entities {
        amount: None,
        source_currency: None,
        target_currency: None,
        recipient_phone: None,
        recipient_address: None,
        card_brand: None,
        contract_address: None,
        network: None,
    }
}

fn first_match<'a>(haystack: &str, needles: &[&'a str]) -> Option<&'a str> {
    needles.iter().copied().find(|n| haystack.to_uppercase().contains(n))
}

fn detect_ccy_before_to(t: &str) -> Option<String> {
    let up = t.to_uppercase();
    KNOWN_CCYS
        .iter()
        .copied()
        .find(|c| up.contains(c))
        .map(str::to_string)
}

fn detect_ccy_after_to(t: &str) -> Option<String> {
    let up = t.to_uppercase();
    let idx = up.rfind(" TO ")?;
    let tail = &up[idx + 3..];
    tail.split_whitespace()
        .filter_map(|w| KNOWN_CCYS.iter().copied().find(|c| w.contains(c)))
        .next()
        .map(str::to_string)
}

pub fn extract_amount(text: &str) -> Option<f64> {
    RE_AMOUNT
        .captures(text)?
        .get(1)?
        .as_str()
        .replace(',', "")
        .parse()
        .ok()
}

fn brand_of(t: &str) -> Option<String> {
    first_match(t, &KNOWN_BRANDS).map(|b| b.replace(' ', "_"))
}

fn network_of(t: &str) -> Option<String> {
    first_match(t, &KNOWN_NETWORKS).map(str::to_string)
}

pub fn rulebook_parse(raw_text: &str) -> crate::parser::ParsedIntent {
    let text = &crate::normalizer::normalize_text(raw_text);
    let amount = extract_amount(text);
    let contract = RE_CONTRACT.find(text).map(|m| m.as_str().to_string());

    let gift_present = RE_GIFT.is_match(text)
        && Regex::new(r"(?i)\b(card|coupon|gift)\b")
            .expect("static pattern")
            .is_match(text);

    let swap_present = RE_SWAP.is_match(text) && text.to_lowercase().contains(" to ");

    let (intent, entities) = if gift_present {
        (
            "LIQUIDATE_GIFT_CARD",
            Entities { card_brand: brand_of(text), amount, ..empty_entities() },
        )
    } else if swap_present {
        (
            "EXECUTE_DEX_SWAP",
            Entities {
                amount,
                source_currency: detect_ccy_before_to(text),
                target_currency: detect_ccy_after_to(text),
                ..empty_entities()
            },
        )
    } else if RE_P2P.is_match(text) {
        (
            "P2P_TRANSFER",
            Entities {
                amount,
                recipient_phone: RE_PHONE.find(text).map(|m| m.as_str().to_string()),
                source_currency: detect_ccy_before_to(text),
                ..empty_entities()
            },
        )
    } else if RE_BALANCE.is_match(text) {
        ("CHECK_BALANCE", empty_entities())
    } else if RE_AIRTIME.is_match(text) {
        (
            "BUY_AIRTIME",
            Entities { amount, network: network_of(text), ..empty_entities() },
        )
    } else if contract.is_some() {
        (
            "CHECK_CONTRACT_SECURITY",
            Entities { contract_address: contract, ..empty_entities() },
        )
    } else {
        ("UNKNOWN", empty_entities())
    };

    crate::parser::ParsedIntent {
        intent: intent.into(),
        confidence: 0.5,
        entities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_truth_send_usdt() {
        let p = rulebook_parse("Send 20 USDT to +2348012345678");
        assert_eq!(p.intent, "P2P_TRANSFER");
        assert_eq!(p.entities.amount, Some(20.0));
        assert_eq!(p.entities.recipient_phone.as_deref(), Some("+2348012345678"));
    }

    #[test]
    fn ground_truth_steam_card() {
        let p = rulebook_parse("Convert 50k Steam card to cash");
        assert_eq!(p.intent, "LIQUIDATE_GIFT_CARD");
        assert_eq!(p.entities.card_brand.as_deref(), Some("STEAM"));
        assert_eq!(p.entities.amount, Some(50000.0));
    }

    #[test]
    fn ground_truth_swap() {
        let p = rulebook_parse("swap 30 usdt to sol");
        assert_eq!(p.intent, "EXECUTE_DEX_SWAP");
        assert_eq!(p.entities.source_currency.as_deref(), Some("USDT"));
        assert_eq!(p.entities.target_currency.as_deref(), Some("SOL"));
        assert_eq!(p.entities.amount, Some(30.0));
    }

    #[test]
    fn ground_truth_balance() {
        assert_eq!(rulebook_parse("wetin dey my wallet balance").intent, "CHECK_BALANCE");
    }

    #[test]
    fn ground_truth_contract_scan() {
        let p = rulebook_parse("check 0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D for honeypot");
        assert_eq!(p.intent, "CHECK_CONTRACT_SECURITY");
        assert!(p.entities.contract_address.is_some());
    }

    #[test]
    fn gibberish_is_unknown() {
        assert_eq!(rulebook_parse("hello how now").intent, "UNKNOWN");
    }

    #[test]
    fn fallback_confidence_is_flagged_low() {
        assert!((rulebook_parse("anything").confidence - 0.5).abs() < f64::EPSILON);
    }
}

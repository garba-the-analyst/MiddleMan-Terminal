use once_cell::sync::Lazy;
use regex::Regex;

static RE_NEG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(urgent|angry|upset|complaint|scam|fraud|delay|pending too long|bad|terrible|awful|hate|refund now|stolen|not working|failed|error)\b").unwrap());
static RE_POS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(thank|thanks|great|good|love|awesome|perfect|appreciate|helpful|quick)\b").unwrap());
static RE_URGENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(urgent|asap|immediately|now|emergency|critical|stuck|blocked|can't access|locked)\b").unwrap());
static RE_EXCLAIM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[!]{2,}|[A-Z]{5,}").unwrap());

#[derive(Debug, Clone, PartialEq)]
pub struct SentimentUrgency {
    pub sentiment: String,      // positive|neutral|negative
    pub urgency: String,        // low|medium|high|critical
    pub urgency_score: i32,     // 0-100
    pub escalation: bool,
    pub escalation_reason: Option<String>,
    pub category: String,       // delivery|payment|refund|complaint|product_enquiry|gift_card|balance|p2p|general|faq
}

pub fn classify_sentiment_urgency(text: &str, intent: &str, confidence: f64) -> SentimentUrgency {
    let lower = text.to_lowercase();
    let is_negative = RE_NEG.is_match(&lower) || RE_EXCLAIM.is_match(text);
    let is_positive = RE_POS.is_match(&lower) && !is_negative;
    let sentiment = if is_negative { "negative" } else if is_positive { "positive" } else { "neutral" }.to_string();

    let mut score: i32 = 20;
    if RE_URGENT.is_match(&lower) { score += 40; }
    if is_negative { score += 25; }
    if text.contains('!') { score += 10; }
    if lower.contains("pending") && lower.contains("since") { score += 20; }
    if confidence < 0.6 { score += 15; }
    score = score.clamp(0, 100);

    let urgency = if score >= 85 { "critical" } else if score >= 65 { "high" } else if score >= 40 { "medium" } else { "low" }.to_string();

    let escalation = (sentiment == "negative" && score >= 65) || confidence < 0.45 || lower.contains("human") || lower.contains("agent") || lower.contains("escalate");
    let escalation_reason = if escalation {
        Some(if sentiment == "negative" && score >= 65 { "Negative sentiment + high urgency".into() }
             else if confidence < 0.45 { "Low confidence — needs human review".into() }
             else { "Explicit escalation request".into() })
    } else { None };

    // Category mapping from intent + keywords (Case Study 1 taxonomy)
    let category = if intent == "LIQUIDATE_GIFT_CARD" { "gift_card" }
        else if intent == "CHECK_BALANCE" { "balance" }
        else if intent == "P2P_TRANSFER" { "p2p" }
        else if intent == "CHECK_CONTRACT_SECURITY" { "security" }
        else if lower.contains("refund") { "refund" }
        else if lower.contains("delivery") || lower.contains("when will") || lower.contains("credited") { "delivery" }
        else if lower.contains("payment") || lower.contains("debited") || lower.contains("failed") { "payment" }
        else if lower.contains("complaint") || is_negative { "complaint" }
        else if lower.contains("product") || lower.contains("ecode") || lower.contains("physical") { "product_enquiry" }
        else if intent == "UNKNOWN" || intent == "HELP" { "faq" }
        else { "general" }.to_string();

    SentimentUrgency { sentiment, urgency, urgency_score: score, escalation, escalation_reason, category }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_negative_high_urgency() {
        let r = classify_sentiment_urgency("My card pending since morning URGENT!!!", "UNKNOWN", 0.5);
        assert_eq!(r.sentiment, "negative");
        assert!(r.urgency_score >= 65);
        assert!(r.escalation);
    }
    #[test]
    fn positive_low() {
        let r = classify_sentiment_urgency("Thanks great service!", "CHECK_BALANCE", 0.9);
        assert_eq!(r.sentiment, "positive");
        assert_eq!(r.urgency, "low");
        assert!(!r.escalation);
    }
}

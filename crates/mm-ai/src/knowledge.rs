use once_cell::sync::Lazy;

/// Simple keyword-based retrieval over knowledge_base (fallback when Gemini unavailable)
/// Production would use pgvector / embeddings; this is deterministic and demo-fast.

#[derive(Debug, Clone)]
pub struct KbArticle {
    pub id: String,
    pub category: String,
    pub question: String,
    pub answer: String,
    pub priority: i32,
}

static ARTICLES: Lazy<Vec<KbArticle>> = Lazy::new(|| vec![
    KbArticle { id: "kb-1".into(), category: "gift_card".into(), question: "What gift cards do you accept and at what rates?".into(), answer: "We accept STEAM ($1450/$), APPLE ($1500/$), AMAZON ($1420/$), RAZER GOLD ($1380/$), GOOGLE PLAY ($1360/$). Rates update daily. Send a clear photo.".into(), priority: 10 },
    KbArticle { id: "kb-2".into(), category: "gift_card".into(), question: "How long does gift card liquidation take?".into(), answer: "Most cards are verified within 15-45 minutes. PHYSICAL cards may take up to 2 hours. You will be notified on WhatsApp.".into(), priority: 9 },
    KbArticle { id: "kb-3".into(), category: "payment".into(), question: "My payment failed but I was debited".into(), answer: "If debited but not credited, it auto-reverses within 24h. Share your transaction ID and we will trace it. Urgent cases escalate to COMPLIANCE.".into(), priority: 10 },
    KbArticle { id: "kb-4".into(), category: "refund".into(), question: "How do I get a refund for a rejected card?".into(), answer: "Rejected cards are not charged. If you were debited, open a dispute and support will refund within 24 hours.".into(), priority: 8 },
    KbArticle { id: "kb-5".into(), category: "delivery".into(), question: "When will my wallet be credited after approval?".into(), answer: "Instantly. Once an agent approves, your NGN wallet is credited and you get a WhatsApp confirmation.".into(), priority: 9 },
    KbArticle { id: "kb-6".into(), category: "wallet".into(), question: "What is the minimum and maximum card value?".into(), answer: "We accept $10 - $2000 per card. Below $10 we cannot process.".into(), priority: 7 },
    KbArticle { id: "kb-7".into(), category: "security".into(), question: "Is my card code safe?".into(), answer: "Yes. Codes are encrypted with AES-256-GCM and auto-deleted after verification.".into(), priority: 8 },
    KbArticle { id: "kb-8".into(), category: "product_enquiry".into(), question: "Do you buy ECODE or PHYSICAL?".into(), answer: "Both. ECODE is online code, PHYSICAL is plastic card photo. ECODE settles faster.".into(), priority: 6 },
    KbArticle { id: "kb-9".into(), category: "complaint".into(), question: "My trade has been pending too long".into(), answer: "High-risk brands take longer. If >2 hours, reply URGENT and we will escalate to Operations Manager.".into(), priority: 9 },
    KbArticle { id: "kb-10".into(), category: "payment".into(), question: "Can I send money to another user?".into(), answer: "Yes: Send 'Send 5000 to 08012345678'. Fee is ₦0 for now.".into(), priority: 7 },
]);

pub fn search_kb(query: &str, category_hint: Option<&str>) -> Option<KbArticle> {
    let q = query.to_lowercase();
    let tokens: Vec<&str> = q.split_whitespace().filter(|t| t.len() > 2).collect();
    if tokens.is_empty() { return None; }
    let mut best: Option<(&KbArticle, i32)> = None;
    for art in ARTICLES.iter() {
        if let Some(cat) = category_hint {
            if art.category != cat && art.priority < 9 { continue; }
        }
        let hay = format!("{} {} {}", art.question, art.answer, art.category).to_lowercase();
        let mut score = 0;
        for tok in &tokens {
            if hay.contains(tok) { score += 10; }
        }
        // boost if category matches
        if let Some(cat) = category_hint { if art.category == cat { score += 5; } }
        score += art.priority;
        if score > 12 {
            if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                best = Some((art, score));
            }
        }
    }
    best.map(|(a,_)| a.clone())
}

pub fn kb_all() -> Vec<KbArticle> { ARTICLES.clone() }

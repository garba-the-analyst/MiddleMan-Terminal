use once_cell::sync::Lazy;
use regex::Regex;

static K_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(\d+(?:\.\d+)?)\s*k\b").unwrap());
static M_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(\d+(?:\.\d+)?)\s*m\b").unwrap());
static NG_PHONE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:\+?234|0)[\s\-]?([7-9]\d{9})").unwrap());

pub fn normalize_text(input: &str) -> String {
    let s = input.trim();
    let s = K_SUFFIX.replace_all(s, |c: &regex::Captures| {
        let whole = c[1].split('.').next().unwrap_or(&c[1]);
        format!("{whole}000")
    });
    let s = M_SUFFIX.replace_all(&s, |c: &regex::Captures| {
        let whole = &c[1];
        match whole.split('.').nth(1) {
            None => format!("{whole}000000"),
            Some(frac) if frac.len() == 1 => {
                let base = whole.split('.').next().unwrap_or("0");
                let first = frac.chars().next().unwrap_or('0');
                format!("{base}{first}00000")
            }
            Some(frac) => {
                let base = whole.split('.').next().unwrap_or("0");
                let first2: String = frac.chars().take(2).collect();
                format!("{base}{first2}0000")
            }
        }
    });
    NG_PHONE
        .replace_all(&s, |c: &regex::Captures| {
            let digits: String = c[1].chars().filter(|ch| ch.is_ascii_digit()).collect();
            format!("+234{digits}")
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_suffix_expands() {
        assert_eq!(normalize_text("send 50k"), "send 50000");
        assert_eq!(normalize_text("50K steam card"), "50000 steam card");
    }

    #[test]
    fn m_suffix_expands() {
        assert_eq!(normalize_text("cashout 2.5m"), "cashout 2500000");
        assert_eq!(normalize_text("2m naira"), "2000000 naira");
    }

    #[test]
    fn nigerian_phones_become_e164() {
        assert_eq!(normalize_text("to 08012345678"), "to +2348012345678");
        assert_eq!(normalize_text("to 2348012345678"), "to +2348012345678");
        assert_eq!(normalize_text("to +234 8012345678"), "to +2348012345678");
    }

    #[test]
    fn plain_numbers_untouched() {
        assert_eq!(normalize_text("swap 50 usdt"), "swap 50 usdt");
        assert_eq!(normalize_text("kola 5 kings"), "kola 5 kings");
    }
}

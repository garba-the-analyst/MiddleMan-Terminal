pub mod fallback;
pub mod normalizer;
pub mod parser;
pub mod radar;
pub mod vision;

pub use fallback::rulebook_parse;
pub use normalizer::normalize_text;
pub use parser::{AiError, GeminiParser, Entities, ParsedIntent};
pub use vision::{read_card_image, CardRead};

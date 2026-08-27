use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("user not found")]
    UserNotFound,
    #[error("insufficient funds")]
    InsufficientFunds,
    #[error("pin locked until {until}")]
    PinLocked { until: String },
    #[error("invalid state transition from {from} on input {input}")]
    InvalidTransition { from: String, input: String },
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_stable() {
        let e = CoreError::PinLocked {
            until: "2026-01-01T00:00:00Z".into(),
        };
        assert_eq!(
            e.to_string(),
            "pin locked until 2026-01-01T00:00:00Z"
        );
        assert_eq!(CoreError::UserNotFound.to_string(), "user not found");
    }
}

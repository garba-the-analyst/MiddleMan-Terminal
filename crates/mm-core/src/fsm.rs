use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlowState {
    Idle,
    Onboarding,
    AwaitingTransactionData,
    AwaitingPin,
    ExecutingAction,
    Notification,
    PinLocked,
}

impl FlowState {
    pub fn as_str(&self) -> &'static str {
        match self {
            FlowState::Idle => "IDLE",
            FlowState::Onboarding => "ONBOARDING",
            FlowState::AwaitingTransactionData => "AWAITING_TRANSACTION_DATA",
            FlowState::AwaitingPin => "AWAITING_PIN",
            FlowState::ExecutingAction => "EXECUTING_ACTION",
            FlowState::Notification => "NOTIFICATION",
            FlowState::PinLocked => "PIN_LOCKED",
        }
    }

    pub fn from_db(raw: &str) -> Self {
        match raw {
            "ONBOARDING" => FlowState::Onboarding,
            "AWAITING_TRANSACTION_DATA" => FlowState::AwaitingTransactionData,
            "AWAITING_PIN" => FlowState::AwaitingPin,
            "EXECUTING_ACTION" => FlowState::ExecutingAction,
            "NOTIFICATION" => FlowState::Notification,
            "PIN_LOCKED" => FlowState::PinLocked,
            _ => FlowState::Idle,
        }
    }

    pub fn is_terminal_for_message(&self) -> bool {
        matches!(self, FlowState::PinLocked)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub from: FlowState,
    pub input: TransitionInput,
    pub to: FlowState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransitionInput {
    IntentParsed,
    ValidDataSubmitted,
    PinValid,
    PinInvalid { strikes: u8 },
    ActionCompleted,
    Timeout,
}

pub const TRANSITIONS: &[Transition] = &[
    Transition { from: FlowState::Idle, input: TransitionInput::IntentParsed, to: FlowState::AwaitingTransactionData },
    Transition { from: FlowState::AwaitingTransactionData, input: TransitionInput::ValidDataSubmitted, to: FlowState::AwaitingPin },
    Transition { from: FlowState::AwaitingPin, input: TransitionInput::PinValid, to: FlowState::ExecutingAction },
    Transition { from: FlowState::AwaitingPin, input: TransitionInput::PinInvalid { strikes: 1 }, to: FlowState::AwaitingPin },
    Transition { from: FlowState::AwaitingPin, input: TransitionInput::PinInvalid { strikes: 3 }, to: FlowState::PinLocked },
    Transition { from: FlowState::ExecutingAction, input: TransitionInput::ActionCompleted, to: FlowState::Notification },
    Transition { from: FlowState::Notification, input: TransitionInput::ActionCompleted, to: FlowState::Idle },
    Transition { from: FlowState::AwaitingTransactionData, input: TransitionInput::Timeout, to: FlowState::Idle },
    Transition { from: FlowState::AwaitingPin, input: TransitionInput::Timeout, to: FlowState::Idle },
    Transition { from: FlowState::PinLocked, input: TransitionInput::Timeout, to: FlowState::Idle },
];

pub fn next_state(current: FlowState, input: &TransitionInput) -> Option<FlowState> {
    TRANSITIONS
        .iter()
        .find(|t| t.from == current && matches_input(&t.input, input))
        .map(|t| t.to)
}

fn matches_input(rule: &TransitionInput, input: &TransitionInput) -> bool {
    match (rule, input) {
        (TransitionInput::PinInvalid { strikes: a }, TransitionInput::PinInvalid { strikes: b }) => a == b,
        (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_walks_the_doctrine_diagram() {
        let s = next_state(FlowState::Idle, &TransitionInput::IntentParsed);
        assert_eq!(s, Some(FlowState::AwaitingTransactionData));

        let s = next_state(s.unwrap(), &TransitionInput::ValidDataSubmitted);
        assert_eq!(s, Some(FlowState::AwaitingPin));

        let s = next_state(s.unwrap(), &TransitionInput::PinValid);
        assert_eq!(s, Some(FlowState::ExecutingAction));

        let s = next_state(s.unwrap(), &TransitionInput::ActionCompleted);
        assert_eq!(s, Some(FlowState::Notification));
    }

    #[test]
    fn three_strikes_locks_the_vault() {
        assert_eq!(
            next_state(FlowState::AwaitingPin, &TransitionInput::PinInvalid { strikes: 1 }),
            Some(FlowState::AwaitingPin)
        );
        assert_eq!(
            next_state(FlowState::AwaitingPin, &TransitionInput::PinInvalid { strikes: 3 }),
            Some(FlowState::PinLocked)
        );
    }

    #[test]
    fn timeouts_return_to_idle() {
        for from in [FlowState::AwaitingTransactionData, FlowState::AwaitingPin] {
            assert_eq!(next_state(from, &TransitionInput::Timeout), Some(FlowState::Idle));
        }
    }

    #[test]
    fn illegal_transition_is_none() {
        assert_eq!(next_state(FlowState::Idle, &TransitionInput::PinValid), None);
    }

    #[test]
    fn db_roundtrip() {
        for state in [
            FlowState::Idle,
            FlowState::Onboarding,
            FlowState::AwaitingTransactionData,
            FlowState::AwaitingPin,
            FlowState::ExecutingAction,
            FlowState::Notification,
            FlowState::PinLocked,
        ] {
            assert_eq!(FlowState::from_db(state.as_str()), state);
        }
        assert_eq!(FlowState::from_db("GARBAGE"), FlowState::Idle);
    }
}

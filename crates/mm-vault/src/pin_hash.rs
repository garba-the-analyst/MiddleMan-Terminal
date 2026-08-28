use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use thiserror::Error;

use crate::kdf;

#[derive(Debug, Error, PartialEq)]
pub enum PinPolicyError {
    #[error("PIN must be 4-6 numeric digits")]
    PolicyViolation,
}

fn argon2_spec() -> Argon2<'static> {
    Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        kdf::spec_params(),
    )
}

fn assert_policy(pin: &str) -> Result<(), PinPolicyError> {
    let ok_len = (4..=6).contains(&pin.len());
    let ok_digits = pin.bytes().all(|b| b.is_ascii_digit());
    if ok_len && ok_digits {
        Ok(())
    } else {
        Err(PinPolicyError::PolicyViolation)
    }
}

pub fn hash_pin(pin: &str) -> Result<String, PinPolicyError> {
    assert_policy(pin)?;
    let salt = SaltString::generate(&mut OsRng);
    argon2_spec()
        .hash_password(pin.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| PinPolicyError::PolicyViolation)
}

pub fn verify_pin(pin: &str, stored_hash: &str) -> Result<bool, argon2::password_hash::Error> {
    let parsed = PasswordHash::new(stored_hash)?;
    Ok(argon2_spec()
        .verify_password(pin.as_bytes(), &parsed)
        .is_ok())
}

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    argon2_spec()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_rejects_non_conforming_pins() {
        assert_eq!(hash_pin("abc"), Err(PinPolicyError::PolicyViolation));
        assert_eq!(hash_pin("123"), Err(PinPolicyError::PolicyViolation));
        assert_eq!(hash_pin("1234567"), Err(PinPolicyError::PolicyViolation));
        assert_eq!(hash_pin("12a4"), Err(PinPolicyError::PolicyViolation));
        assert!(hash_pin("1234").is_ok());
        assert!(hash_pin("123456").is_ok());
    }

    #[test]
    fn phc_string_carries_spec_parameters() {
        let h = hash_pin("2468").unwrap();
        assert!(
            h.starts_with("$argon2id$v=19$m=65536,t=3,p=4$"),
            "unexpected params in {h}"
        );
    }

    #[test]
    fn verify_roundtrip() {
        let h = hash_pin("9091").unwrap();
        assert!(verify_pin("9091", &h).unwrap());
        assert!(!verify_pin("9092", &h).unwrap());
    }

    #[test]
    fn malformed_hash_is_error_not_panic() {
        assert!(verify_pin("1234", "not-a-hash").is_err());
    }
}

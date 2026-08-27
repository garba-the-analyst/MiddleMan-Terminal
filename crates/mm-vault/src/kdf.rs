use argon2::{Algorithm, Argon2, Params, Version};

pub const PIN_MEMORY_KIB: u32 = 65536;
pub const PIN_ITERATIONS: u32 = 3;
pub const PIN_PARALLELISM: u32 = 4;

pub fn spec_params() -> Params {
    Params::new(PIN_MEMORY_KIB, PIN_ITERATIONS, PIN_PARALLELISM, Some(32))
        .expect("spec params are valid")
}

pub fn derive_pin_key(pin: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, spec_params());
    argon2
        .hash_password_into(pin.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_is_deterministic_per_salt() {
        let a = derive_pin_key("1234", b"sixteen-byte-salt").unwrap();
        let b = derive_pin_key("1234", b"sixteen-byte-salt").unwrap();
        let c = derive_pin_key("1235", b"sixteen-byte-salt").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}

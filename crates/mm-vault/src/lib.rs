pub mod aead;
pub mod kdf;
pub mod pin_hash;
pub mod sensitive;

pub use aead::{VaultAead, VaultError};
pub use pin_hash::{hash_pin, verify_pin};
pub use sensitive::SensitiveKeyBuffer;

pub fn derive_subkeys(master: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let ikm = hkdf::Hkdf::<sha2::Sha256>::new(None, master);
    let mut vault_keys = [0u8; 32];
    let mut hmac_bvn = [0u8; 32];
    ikm.expand(b"mm/vault/keys/v1", &mut vault_keys)
        .expect("hkdf length valid");
    ikm.expand(b"mm/hmac/bvn/v1", &mut hmac_bvn)
        .expect("hkdf length valid");
    (vault_keys, hmac_bvn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subkeys_are_deterministic_and_domain_separated() {
        let master = [7u8; 32];
        let (a1, b1) = derive_subkeys(&master);
        let (a2, b2) = derive_subkeys(&master);
        assert_eq!(a1, a2);
        assert_eq!(b1, b2);
        assert_ne!(a1, b1);
    }
}

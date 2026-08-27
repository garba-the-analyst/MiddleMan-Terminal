use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand_core::{OsRng, RngCore};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum VaultError {
    #[error("master key must be 64 hex chars decoding to 32 bytes")]
    BadMasterKey,
    #[error("ciphertext malformed")]
    Malformed,
    #[error("authentication failed (wrong key or tampered payload)")]
    AuthFailed,
}

const VERSION_V1: u8 = 0x01;

pub struct VaultAead {
    cipher: Aes256Gcm,
}

impl VaultAead {
    pub fn from_hex_master(hex_master: &str) -> Result<Self, VaultError> {
        let bytes = hex::decode(hex_master.trim()).map_err(|_| VaultError::BadMasterKey)?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| VaultError::BadMasterKey)?;
        let (vault_keys, _) = crate::derive_subkeys(&key);
        Ok(Self {
            cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&vault_keys)),
        })
    }

    pub fn encrypt(&self, plaintext: &[u8], aad: &str) -> Result<String, VaultError> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ct = self
            .cipher
            .encrypt(nonce, Payload { msg: plaintext, aad: aad.as_bytes() })
            .map_err(|_| VaultError::AuthFailed)?;

        let mut out = Vec::with_capacity(13 + ct.len());
        out.push(VERSION_V1);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(STANDARD.encode(out))
    }

    pub fn decrypt(&self, envelope_b64: &str, aad: &str) -> Result<Vec<u8>, VaultError> {
        let raw = STANDARD.decode(envelope_b64).map_err(|_| VaultError::Malformed)?;
        if raw.len() < 14 || raw[0] != VERSION_V1 {
            return Err(VaultError::Malformed);
        }
        let (nonce_bytes, ct) = raw[1..].split_at(12);
        self.cipher
            .decrypt(Nonce::from_slice(nonce_bytes), Payload { msg: ct, aad: aad.as_bytes() })
            .map_err(|_| VaultError::AuthFailed)
    }

    pub fn decrypt_sensitive(
        &self,
        envelope_b64: &str,
        aad: &str,
    ) -> Result<crate::sensitive::SensitiveKeyBuffer, VaultError> {
        Ok(crate::sensitive::SensitiveKeyBuffer::new(
            self.decrypt(envelope_b64, aad)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "3cfa4ff0a1e2d3c4b5a69788796a5b4c3d2e1f00918273645463728190a0b1c2";

    fn vault() -> VaultAead {
        VaultAead::from_hex_master(MASTER).unwrap()
    }

    #[test]
    fn rejects_bad_master_keys() {
        assert_eq!(
            VaultAead::from_hex_master("short").map(|_| ()),
            Err(VaultError::BadMasterKey)
        );
        assert_eq!(
            VaultAead::from_hex_master(
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
            )
            .map(|_| ()),
            Err(VaultError::BadMasterKey)
        );
    }

    #[test]
    fn roundtrip_is_exact() {
        let v = vault();
        let secret = b"solana-secret-key-bytes-0001";
        let envelope = v.encrypt(secret, "mm:vault:v1:user-1:SOLANA").unwrap();
        let plain = v.decrypt(&envelope, "mm:vault:v1:user-1:SOLANA").unwrap();
        assert_eq!(plain, secret);
    }

    #[test]
    fn aad_binding_prevents_row_swapping() {
        let v = vault();
        let envelope = v.encrypt(b"key", "mm:vault:v1:user-1:SOLANA").unwrap();
        assert_eq!(
            v.decrypt(&envelope, "mm:vault:v1:user-2:SOLANA"),
            Err(VaultError::AuthFailed)
        );
    }

    #[test]
    fn tampered_ciphertext_fails_auth() {
        use base64::Engine as _;
        let v = vault();
        let envelope = v.encrypt(b"key", "aad").unwrap();
        let mut raw = STANDARD.decode(envelope).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        let tampered = STANDARD.encode(raw);
        assert_eq!(v.decrypt(&tampered, "aad"), Err(VaultError::AuthFailed));
    }

    #[test]
    fn wrong_version_or_short_payload_is_malformed() {
        use base64::Engine as _;
        let v = vault();
        assert_eq!(v.decrypt(&STANDARD.encode([0u8; 13]), "aad"), Err(VaultError::Malformed));
        let mut wrong_version = vec![0x02u8];
        wrong_version.extend_from_slice(&[0u8; 20]);
        assert_eq!(v.decrypt(&STANDARD.encode(wrong_version), "aad"), Err(VaultError::Malformed));
    }

    #[test]
    fn sensitive_buffer_roundtrip() {
        let v = vault();
        let envelope = v.encrypt(b"private-key", "aad-x").unwrap();
        let sensitive = v.decrypt_sensitive(&envelope, "aad-x").unwrap();
        assert_eq!(sensitive.decrypted_private_key, b"private-key");
    }
}

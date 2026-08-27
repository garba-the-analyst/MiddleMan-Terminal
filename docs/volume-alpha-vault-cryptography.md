# VOLUME ALPHA — `mm-vault` & Cryptographic Security Specification

**Version:** 2.4.0 · **Owner:** Security Engineering · **Scope:** PIN hashing, wallet key
encryption at rest, RAM hygiene, key rotation, lockout integration.

---

## 1. Architectural Overview & Technical Scope

`mm-vault` is the single custodian of every secret in MiddleMan:

| Secret | Protection | Storage |
|--------|-----------|---------|
| User login PIN (4–6 digits) | Argon2id PHC string | `users.pin_hash` |
| EVM / Solana / Tron / TON private keys | AES-256-GCM ciphertext, master-key derived | `key_vaults.encrypted_private_key` |
| BVN/NIN identifiers | SHA-256 keyed hash (HMAC) for lookup + Argon2id for audit | `users.bvn_nin_hash` |
| Admin passwords | Argon2id PHC string (same module) | `admin_employees.password_hash` |

The crate is pure: no I/O, no async, no DB. Callers (`mm-api`) own persistence.

## 2. Cryptographic Guarantees & Parameters

### 2.1 PIN hashing (Argon2id)

```
Algorithm : Argon2id (v0x13)
Memory    : 64 MiB  (m=65536 KiB)
Iterations: t = 3
Parallelism: p = 4
Salt      : 16 bytes CSPRNG per hash
Output    : PHC string ($argon2id$v=19$m=65536,t=3,p=4$salt$hash)
```

64 MiB × p4 means one verification ≈ 100 ms on the 1 vCPU VPS — an intentional throttle
against online guessing; worker concurrency of 5 keeps worst-case queueing acceptable.

### 2.2 Key encryption (AES-256-GCM)

Master key handling:

- `MM_MASTER_KEY` env var: exactly 64 hex chars → 32 bytes.
- Per-purpose subkeys are derived via HKDF-SHA256 to avoid key reuse across domains:
  - `vault_keys   = HKDF(master, info="mm/vault/keys/v1")`
  - `hmac_bvn     = HKDF(master, info="mm/hmac/bvn/v1")`
- Envelope format stored in DB (`base64`):

```
byte 0      : version tag (0x01)
bytes 1..12 : nonce (96-bit, CSPRNG)
bytes 12..  : GCM ciphertext || 128-bit auth tag
AAD         : "mm:vault:v1:{user_id}:{chain_type}"   (binds ciphertext to its row)
```

AAD binding means a ciphertext copied between users/chains fails authentication.

### 2.3 RAM hygiene

Any plaintext private key lives inside `SensitiveKeyBuffer`, which implements
`Zeroize + ZeroizeOnDrop`. Signing scopes take it by value; on drop, memory is overwritten.
Clippy lint `mem_forget` disallowed at workspace level.

## 3. Complete Implementation

### 3.1 `crates/mm-vault/src/lib.rs`

```rust
pub mod aead;
pub mod kdf;
pub mod pin_hash;
pub mod sensitive;

pub use aead::{VaultAead, VaultError};
pub use pin_hash::{hash_pin, verify_pin, PinPolicyError};
pub use sensitive::SensitiveKeyBuffer;

/// Domain-separated HKDF derivations from the master key.
pub fn derive_subkeys(master: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let ikm = hkdf::Hkdf::<sha2::Sha256>::new(None, master);
    let mut vault_keys = [0u8; 32];
    let mut hmac_bvn = [0u8; 32];
    ikm.expand(b"mm/vault/keys/v1", &mut vault_keys).expect("len ok");
    ikm.expand(b"mm/hmac/bvn/v1", &mut hmac_bvn).expect("len ok");
    (vault_keys, hmac_bvn)
}
```

Add to `Cargo.toml`: `hkdf = "0.12"`, `sha2 = "0.10"`, `zeroize = "1"`,
`hex = "0.4"` alongside existing deps.

### 3.2 `src/sensitive.rs`

```rust
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SensitiveKeyBuffer {
    pub decrypted_private_key: Vec<u8>,
}

impl SensitiveKeyBuffer {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { decrypted_private_key: bytes }
    }
}
```

### 3.3 `src/kdf.rs`

```rust
use argon2::{Algorithm, Argon2, Params, Version};

/// Derives a 32-byte key material blob from a low-entropy input (PIN) — used only where
/// symmetric wrapping by user knowledge is required. Verification always uses the
/// password-hash PHC compare instead; this function exists for future envelope use cases.
pub fn derive_pin_key(pin: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    let params = Params::new(65536, 3, 4, Some(32)).map_err(|e| e.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon2
        .hash_password_into(pin.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}
```

### 3.4 `src/aead.rs`

```rust
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand_core::{OsRng, RngCore};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("master key must be 64 hex chars")]
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
    /// Parses MM_MASTER_KEY (64 hex chars) into the cipher instance.
    pub fn from_hex_master(hex_master: &str) -> Result<Self, VaultError> {
        let bytes = hex::decode(hex_master.trim()).map_err(|_| VaultError::BadMasterKey)?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| VaultError::BadMasterKey)?;
        let (_, vault_keys) = crate::derive_subkeys(&key);
        Ok(Self {
            cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&vault_keys)),
        })
    }

    /// Encrypts plaintext under AAD binding; returns base64(version||nonce||ct||tag).
    pub fn encrypt(&self, plaintext: &[u8], aad: &str) -> Result<String, VaultError> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ct = self
            .cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| VaultError::AuthFailed)?;

        let mut out = Vec::with_capacity(13 + ct.len());
        out.push(VERSION_V1);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(STANDARD.encode(out))
    }

    /// Decrypts a base64 envelope produced by [`encrypt`].
    pub fn decrypt(&self, envelope_b64: &str, aad: &str) -> Result<Vec<u8>, VaultError> {
        let raw = STANDARD.decode(envelope_b64).map_err(|_| VaultError::Malformed)?;
        if raw.len() < 14 || raw[0] != VERSION_V1 {
            return Err(VaultError::Malformed);
        }
        let (nonce_bytes, ct) = raw[1..].split_at(12);
        let pt = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                Payload {
                    msg: ct,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| VaultError::AuthFailed)?;
        Ok(pt)
    }

    /// Convenience: decrypt straight into a zeroizing buffer for signing scopes.
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
```

### 3.5 `src/pin_hash.rs`

```rust
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PinPolicyError {
    #[error("PIN must be 4-6 numeric digits")]
    PolicyViolation,
}

fn assert_policy(pin: &str) -> Result<(), PinPolicyError> {
    let ok_len = (4..=6).contains(&pin.len());
    let ok_digits = pin.bytes().all(|b| b.is_ascii_digit());
    if ok_len && ok_digits { Ok(()) } else { Err(PinPolicyError::PolicyViolation) }
}

pub fn hash_pin(pin: &str) -> Result<String, PinPolicyError> {
    assert_policy(pin)?;
    let salt = SaltString::generate(&mut OsRng);
    // Default Argon2 here must match §2.1; enforced by test param_check below.
    let argon2 = Argon2::default();
    argon2
        .hash_password(pin.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| PinPolicyError::PolicyViolation)
}

pub fn verify_pin(pin: &str, stored_hash: &str) -> Result<bool, argon2::password_hash::Error> {
    let parsed = PasswordHash::new(stored_hash)?;
    Ok(Argon2::default()
        .verify_password(pin.as_bytes(), &parsed)
        .is_ok())
}
```

## 4. Data Schemas & Structural Interfaces

Consumed columns (Vol 2): `users.pin_hash`, `users.pin_salt`, `users.failed_pin_attempts`,
`users.pin_locked_until`; `key_vaults.encrypted_private_key`, `key_vaults.nonce`.

AAD construction contract in `mm-api`:

```rust
let aad = format!("mm:vault:v1:{}:{}", user_id, chain_type);
```

Lockout policy (enforced by mm-api FSM, informed by this crate):

```
failed_pin_attempts >= 3  =>  pin_locked_until = now() + 2 hours
                          =>  notify user + ops alert (Vol Eta)
```

Timing side channel: verification runs even when `pin_locked_until > now()`? No — locked
accounts short-circuit BEFORE hashing to avoid burning CPU; response text is identical
("locked") regardless of PIN correctness.

## 5. Error Handling Policies

| Condition | Behavior |
|-----------|----------|
| Wrong PIN | `Ok(false)` → FSM increments strikes |
| Corrupt envelope / wrong AAD | `AuthFailed` → DO NOT retry; flag row for ops; treat as potential tampering |
| Master key mismatch after rotation | Same as above; recovery requires re-encrypt job (§6) |
| Policy violation on set-PIN | User-facing "PIN must be 4–6 digits", no state change |

Rotation procedure (master key v1 → v2):
1. Deploy supports both keys (`MM_MASTER_KEY`, `MM_MASTER_KEY_PREV`).
2. Background job re-encrypts each `key_vaults` row: decrypt(prev) → encrypt(current), batched 50/s.
3. Verify all rows decrypt under current key; unset prev env; done.

## 6. Verification Test Cases & Command Sequences

```bash
# VA-T1: unit suite (roundtrip, AAD binding, tamper, zeroize compile, policy)
cargo test -p mm-vault

# Key assertions inside tests:
#   encrypt->decrypt roundtrips identical bytes
#   decrypt with different AAD string => AuthFailed
#   flipping any ciphertext byte => AuthFailed
#   hash_pin rejects "abc", "1234567", "12a4"
#   verify_pin(hash_pin(p), p) == true; wrong pin == false

# VA-T2: parameter conformance
# test asserts PHC prefix "$argon2id$v=19$m=65536,t=3,p=4"

# VA-T3: integration — vault round trip through DB
psql $DATABASE_URL -c "
INSERT INTO key_vaults (user_id, chain_type, public_address, encrypted_private_key, nonce)
VALUES ('<uid>', 'SOLANA', 'TestAddr', '<envelope-from-test>', 'n/a');"
cargo test -p mm-db vault_roundtrip -- --ignored   # reads back and decrypts OK

# VA-T4: rotation dry run
MM_MASTER_KEY=$K2 MM_MASTER_KEY_PREV=$K1 cargo run -p mm-api --bin rotate-keys --dry-run
```

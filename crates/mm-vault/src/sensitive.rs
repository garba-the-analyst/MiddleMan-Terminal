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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroize_scrubs_buffers_in_place() {
        let mut secret = [0xA5u8; 64];
        Zeroize::zeroize(&mut secret);
        assert!(secret.iter().all(|&b| b == 0));
    }

    #[test]
    fn sensitive_buffer_is_zeroizing_on_drop_by_type_contract() {
        fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<SensitiveKeyBuffer>();

        let buf = SensitiveKeyBuffer::new(vec![1u8; 32]);
        let mut scrubbed = buf.clone();
        Zeroize::zeroize(&mut scrubbed);
        assert!(scrubbed.decrypted_private_key.iter().all(|&b| b == 0));
    }
}

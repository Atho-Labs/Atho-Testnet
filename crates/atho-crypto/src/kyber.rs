use crate::error::CryptoError;
use crate::secret::SecretBytes;

pub const KYBER512_PUBLIC_KEY_BYTES: usize = 800;
pub const KYBER512_SECRET_KEY_BYTES: usize = 1_632;
pub const KYBER768_PUBLIC_KEY_BYTES: usize = 1_184;
pub const KYBER768_SECRET_KEY_BYTES: usize = 2_400;
pub const KYBER1024_PUBLIC_KEY_BYTES: usize = 1_568;
pub const KYBER1024_SECRET_KEY_BYTES: usize = 3_168;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KyberVariant {
    Kyber512,
    Kyber768,
    Kyber1024,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KyberPublicKey(pub Vec<u8>);

#[derive(Debug, PartialEq, Eq)]
pub struct KyberSecretKey(pub SecretBytes);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KyberCiphertext(pub Vec<u8>);

#[derive(Debug, PartialEq, Eq)]
pub struct KyberLockbox {
    pub public_key: KyberPublicKey,
    pub secret_key: KyberSecretKey,
    pub variant: KyberVariant,
}

pub fn available() -> bool {
    false
}

pub fn validate_key_lengths(
    public_key: &[u8],
    secret_key: &[u8],
    variant: KyberVariant,
) -> Result<(), CryptoError> {
    let (pk_len, sk_len) = match variant {
        KyberVariant::Kyber512 => (KYBER512_PUBLIC_KEY_BYTES, KYBER512_SECRET_KEY_BYTES),
        KyberVariant::Kyber768 => (KYBER768_PUBLIC_KEY_BYTES, KYBER768_SECRET_KEY_BYTES),
        KyberVariant::Kyber1024 => (KYBER1024_PUBLIC_KEY_BYTES, KYBER1024_SECRET_KEY_BYTES),
    };

    if public_key.len() != pk_len || secret_key.len() != sk_len {
        Err(CryptoError::InvalidKeyLength)
    } else {
        Ok(())
    }
}

pub fn default_variant() -> KyberVariant {
    KyberVariant::Kyber512
}

pub fn wrap(
    _variant: KyberVariant,
    _public_key: &KyberPublicKey,
    _plaintext: &[u8],
) -> Result<KyberCiphertext, CryptoError> {
    Err(CryptoError::BackendUnavailable)
}

pub fn unwrap(
    _lockbox: &KyberLockbox,
    _ciphertext: &KyberCiphertext,
) -> Result<SecretBytes, CryptoError> {
    Err(CryptoError::BackendUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kyber_defaults_to_512() {
        assert_eq!(default_variant(), KyberVariant::Kyber512);
    }

    #[test]
    fn kyber_length_validation_rejects_wrong_sizes() {
        let err = validate_key_lengths(&[], &[], KyberVariant::Kyber512).unwrap_err();
        assert_eq!(err, CryptoError::InvalidKeyLength);
    }
}

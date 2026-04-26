use crate::error::CryptoError;
use crate::secret::SecretBytes;
use atho_core::crypto::hash::sha3_384;
use getrandom::getrandom;
use std::ffi::c_void;

pub const FALCON_512_PUBLIC_KEY_BYTES: usize = 897;
pub const FALCON_512_SECRET_KEY_BYTES: usize = 1_281;
pub const FALCON_512_SIGNATURE_MIN_BYTES: usize = 600;
pub const FALCON_512_SIGNATURE_MAX_BYTES: usize = 752;
pub const FALCON_512_TMP_KEYGEN_BYTES: usize = 65_536;
pub const FALCON_512_TMP_SIGN_BYTES: usize = 65_536;
pub const FALCON_512_TMP_VERIFY_BYTES: usize = 8_192;
const FALCON_512_LOGN: u32 = 9;
const FALCON_SIG_COMPRESSED: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct Shake256Context {
    opaque_contents: [u64; 26],
}

unsafe extern "C" {
    fn shake256_init_prng_from_seed(sc: *mut Shake256Context, seed: *const c_void, seed_len: usize);
    fn falcon_keygen_make(
        rng: *mut Shake256Context,
        logn: u32,
        privkey: *mut c_void,
        privkey_len: usize,
        pubkey: *mut c_void,
        pubkey_len: usize,
        tmp: *mut c_void,
        tmp_len: usize,
    ) -> i32;
    fn falcon_sign_dyn(
        rng: *mut Shake256Context,
        sig: *mut c_void,
        sig_len: *mut usize,
        sig_type: i32,
        privkey: *const c_void,
        privkey_len: usize,
        data: *const c_void,
        data_len: usize,
        tmp: *mut c_void,
        tmp_len: usize,
    ) -> i32;
    fn falcon_verify(
        sig: *const c_void,
        sig_len: usize,
        sig_type: i32,
        pubkey: *const c_void,
        pubkey_len: usize,
        data: *const c_void,
        data_len: usize,
        tmp: *mut c_void,
        tmp_len: usize,
    ) -> i32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FalconVariant {
    Falcon512,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalconPublicKey(pub Vec<u8>);

impl FalconPublicKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct FalconSecretKey(pub SecretBytes);

impl FalconSecretKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0 .0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalconSignature(pub Vec<u8>);

impl FalconSignature {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct FalconKeypair {
    pub public_key: FalconPublicKey,
    pub secret_key: FalconSecretKey,
}

pub fn available() -> bool {
    true
}

pub fn validate_key_lengths(
    public_key: &[u8],
    secret_key: &[u8],
    variant: FalconVariant,
) -> Result<(), CryptoError> {
    match variant {
        FalconVariant::Falcon512 => {
            if public_key.len() != FALCON_512_PUBLIC_KEY_BYTES
                || secret_key.len() != FALCON_512_SECRET_KEY_BYTES
            {
                Err(CryptoError::InvalidKeyLength)
            } else {
                Ok(())
            }
        }
    }
}

pub fn public_key_len_ok(len: usize) -> bool {
    len == FALCON_512_PUBLIC_KEY_BYTES
}

pub fn signature_len_ok(len: usize) -> bool {
    (FALCON_512_SIGNATURE_MIN_BYTES..=FALCON_512_SIGNATURE_MAX_BYTES).contains(&len)
}

fn falcon_seed(seed: &[u8]) -> [u8; 48] {
    let mut out = [0u8; 48];
    if seed.len() == 48 {
        out.copy_from_slice(seed);
    } else {
        out.copy_from_slice(&sha3_384(seed));
    }
    out
}

fn init_rng(seed: &[u8]) -> Shake256Context {
    let mut rng = unsafe { std::mem::zeroed::<Shake256Context>() };
    let seed = falcon_seed(seed);
    unsafe {
        shake256_init_prng_from_seed(&mut rng, seed.as_ptr().cast(), seed.len());
    }
    rng
}

pub fn generate_from_seed(seed: &[u8]) -> Result<FalconKeypair, CryptoError> {
    if seed.is_empty() {
        return Err(CryptoError::InvalidKeyLength);
    }

    let mut rng = init_rng(seed);
    let mut public_key = vec![0u8; FALCON_512_PUBLIC_KEY_BYTES];
    let mut secret_key = vec![0u8; FALCON_512_SECRET_KEY_BYTES];
    let mut tmp = vec![0u8; FALCON_512_TMP_KEYGEN_BYTES];
    let rc = unsafe {
        falcon_keygen_make(
            &mut rng,
            FALCON_512_LOGN,
            secret_key.as_mut_ptr().cast(),
            secret_key.len(),
            public_key.as_mut_ptr().cast(),
            public_key.len(),
            tmp.as_mut_ptr().cast(),
            tmp.len(),
        )
    };
    tmp.fill(0);
    if rc != 0 {
        return Err(CryptoError::OperationFailed);
    }

    Ok(FalconKeypair {
        public_key: FalconPublicKey(public_key),
        secret_key: FalconSecretKey(SecretBytes(secret_key)),
    })
}

pub fn generate(variant: FalconVariant) -> Result<FalconKeypair, CryptoError> {
    match variant {
        FalconVariant::Falcon512 => {
            let mut seed = [0u8; 48];
            getrandom(&mut seed).map_err(|_| CryptoError::BackendUnavailable)?;
            generate_from_seed(&seed)
        }
    }
}

pub fn sign(secret_key: &FalconSecretKey, message: &[u8]) -> Result<FalconSignature, CryptoError> {
    if secret_key.as_bytes().len() != FALCON_512_SECRET_KEY_BYTES {
        return Err(CryptoError::InvalidKeyLength);
    }

    let mut seed = [0u8; 48];
    getrandom(&mut seed).map_err(|_| CryptoError::BackendUnavailable)?;
    let mut rng = init_rng(&seed);
    let mut signature = vec![0u8; FALCON_512_SIGNATURE_MAX_BYTES];
    let mut sig_len = signature.len();
    let mut tmp = vec![0u8; FALCON_512_TMP_SIGN_BYTES];
    let rc = unsafe {
        falcon_sign_dyn(
            &mut rng,
            signature.as_mut_ptr().cast(),
            &mut sig_len,
            FALCON_SIG_COMPRESSED,
            secret_key.as_bytes().as_ptr().cast(),
            secret_key.as_bytes().len(),
            message.as_ptr().cast(),
            message.len(),
            tmp.as_mut_ptr().cast(),
            tmp.len(),
        )
    };
    tmp.fill(0);
    if rc != 0 {
        return Err(CryptoError::OperationFailed);
    }
    signature.truncate(sig_len);
    Ok(FalconSignature(signature))
}

pub fn verify(
    public_key: &FalconPublicKey,
    message: &[u8],
    signature: &FalconSignature,
) -> Result<bool, CryptoError> {
    if !public_key_len_ok(public_key.as_bytes().len())
        || !signature_len_ok(signature.as_bytes().len())
    {
        return Ok(false);
    }

    let mut tmp = vec![0u8; FALCON_512_TMP_VERIFY_BYTES];
    let rc = unsafe {
        falcon_verify(
            signature.as_bytes().as_ptr().cast(),
            signature.as_bytes().len(),
            0,
            public_key.as_bytes().as_ptr().cast(),
            public_key.as_bytes().len(),
            message.as_ptr().cast(),
            message.len(),
            tmp.as_mut_ptr().cast(),
            tmp.len(),
        )
    };
    tmp.fill(0);
    Ok(rc == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falcon512_lengths_are_frozen() {
        assert_eq!(FALCON_512_PUBLIC_KEY_BYTES, 897);
        assert_eq!(FALCON_512_SECRET_KEY_BYTES, 1_281);
        assert_eq!(FALCON_512_SIGNATURE_MIN_BYTES, 600);
        assert_eq!(FALCON_512_SIGNATURE_MAX_BYTES, 752);
        assert_eq!(FALCON_512_TMP_KEYGEN_BYTES, 65_536);
        assert_eq!(FALCON_512_TMP_SIGN_BYTES, 65_536);
        assert_eq!(FALCON_512_TMP_VERIFY_BYTES, 8_192);
    }

    #[test]
    fn falcon_keygen_sign_and_verify_round_trip() {
        let keypair = generate_from_seed(b"atho-falcon-seed").unwrap();
        let message = b"atho signing message";
        let signature = sign(&keypair.secret_key, message).unwrap();
        assert!(verify(&keypair.public_key, message, &signature).unwrap());
        assert!(!verify(&keypair.public_key, b"wrong message", &signature).unwrap());
    }

    #[test]
    fn falcon_length_validation_rejects_wrong_sizes() {
        let err = validate_key_lengths(&[], &[], FalconVariant::Falcon512).unwrap_err();
        assert_eq!(err, CryptoError::InvalidKeyLength);
    }
}

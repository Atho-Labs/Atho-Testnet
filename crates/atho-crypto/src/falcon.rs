// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Falcon-512 key generation and signature verification for Atho.
//!
//! This module wraps the `fn-dsa` Falcon implementation with Atho-specific
//! domain separation and secret-handling helpers.
//!
//! CONSENSUS: Signature verification must stay on the CPU canonical path. Any
//! accelerated backend must produce identical accept/reject results.
use crate::error::CryptoError;
use crate::secret::SecretBytes;
use atho_core::consensus::signatures::AthoSignatureDomain;
use atho_core::crypto::hash::sha3_384;
use fn_dsa::{
    sign_key_size, signature_size, vrfy_key_size, CryptoRng, DomainContext, KeyPairGenerator,
    KeyPairGenerator512, RngCore, SigningKey, SigningKey512, VerifyingKey, VerifyingKey512,
    FN_DSA_LOGN_512, HASH_ID_SHA3_384,
};
use getrandom::getrandom;
use std::cmp::min;
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

pub const FALCON_512_LOGN: u32 = FN_DSA_LOGN_512;
pub const FALCON_512_PUBLIC_KEY_BYTES: usize = vrfy_key_size(FALCON_512_LOGN);
pub const FALCON_512_SECRET_KEY_BYTES: usize = sign_key_size(FALCON_512_LOGN);
pub const FALCON_512_SIGNATURE_BYTES: usize = signature_size(FALCON_512_LOGN);

/// Falcon-512 public verification key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalconPublicKey(pub Vec<u8>);

impl FalconPublicKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        decode_verifying_key(bytes)?;
        Ok(Self(bytes.to_vec()))
    }
}

/// Falcon-512 secret signing key stored in zeroizing memory.
#[derive(PartialEq, Eq)]
pub struct FalconSecretKey(pub SecretBytes);

impl FalconSecretKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0 .0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        decode_signing_key(bytes)?;
        Ok(Self(SecretBytes(bytes.to_vec())))
    }
}

/// Falcon-512 signature bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalconSignature(pub Vec<u8>);

impl FalconSignature {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if !signature_len_ok(bytes.len()) {
            return Err(CryptoError::InvalidKeyLength);
        }
        Ok(Self(bytes.to_vec()))
    }
}

/// Public/secret Falcon keypair used by wallet code and tests.
#[derive(PartialEq, Eq)]
pub struct FalconKeypair {
    pub public_key: FalconPublicKey,
    pub secret_key: FalconSecretKey,
}

impl fmt::Debug for FalconSecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FalconSecretKey(<redacted>)")
    }
}

impl fmt::Debug for FalconKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FalconKeypair")
            .field("public_key", &self.public_key)
            .field("secret_key", &self.secret_key)
            .finish()
    }
}

#[derive(Clone)]
struct SeededRng {
    seed: [u8; 48],
    buffer: [u8; 48],
    offset: usize,
    counter: u64,
}

impl SeededRng {
    fn new(seed: &[u8]) -> Self {
        Self {
            seed: falcon_seed(seed),
            buffer: [0u8; 48],
            offset: 48,
            counter: 0,
        }
    }

    fn refill(&mut self) {
        let mut input = [0u8; 56];
        input[..48].copy_from_slice(&self.seed);
        input[48..].copy_from_slice(&self.counter.to_le_bytes());
        self.buffer = sha3_384(&input);
        self.offset = 0;
        self.counter = self.counter.wrapping_add(1);
    }

    fn clear(&mut self) {
        self.seed.zeroize();
        self.buffer.zeroize();
        self.offset = self.buffer.len();
        self.counter = 0;
    }
}

impl CryptoRng for SeededRng {}

impl RngCore for SeededRng {
    fn next_u32(&mut self) -> u32 {
        if self.offset > self.buffer.len().saturating_sub(4) {
            let mut bytes = [0u8; 4];
            self.fill_bytes(&mut bytes);
            return u32::from_le_bytes(bytes);
        }

        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&self.buffer[self.offset..self.offset + 4]);
        self.offset += 4;
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        if self.offset > self.buffer.len().saturating_sub(8) {
            let mut bytes = [0u8; 8];
            self.fill_bytes(&mut bytes);
            return u64::from_le_bytes(bytes);
        }

        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.buffer[self.offset..self.offset + 8]);
        self.offset += 8;
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut filled = 0usize;
        while filled < dest.len() {
            if self.offset >= self.buffer.len() {
                self.refill();
            }
            let available = self.buffer.len() - self.offset;
            let remaining = dest.len() - filled;
            let count = min(available, remaining);
            dest[filled..filled + count]
                .copy_from_slice(&self.buffer[self.offset..self.offset + count]);
            self.offset += count;
            filled += count;
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), fn_dsa::RngError> {
        self.fill_bytes(dest);
        Ok(())
    }
}

/// Returns whether the Falcon backend is available in this build.
pub fn available() -> bool {
    true
}

/// Validates a Falcon public key length.
pub fn public_key_len_ok(len: usize) -> bool {
    len == FALCON_512_PUBLIC_KEY_BYTES
}

/// Validates a Falcon secret key length.
pub fn secret_key_len_ok(len: usize) -> bool {
    len == FALCON_512_SECRET_KEY_BYTES
}

/// Validates a Falcon signature length.
pub fn signature_len_ok(len: usize) -> bool {
    len == FALCON_512_SIGNATURE_BYTES
}

/// Verifies that the provided raw key lengths match the Falcon-512 profile.
pub fn validate_key_lengths(public_key: &[u8], secret_key: &[u8]) -> Result<(), CryptoError> {
    if !public_key_len_ok(public_key.len()) || !secret_key_len_ok(secret_key.len()) {
        Err(CryptoError::InvalidKeyLength)
    } else {
        Ok(())
    }
}

fn decode_signing_key(bytes: &[u8]) -> Result<SigningKey512, CryptoError> {
    if !secret_key_len_ok(bytes.len()) {
        return Err(CryptoError::InvalidKeyLength);
    }
    SigningKey512::decode(bytes).ok_or(CryptoError::OperationFailed)
}

fn decode_verifying_key(bytes: &[u8]) -> Result<VerifyingKey512, CryptoError> {
    if !public_key_len_ok(bytes.len()) {
        return Err(CryptoError::InvalidKeyLength);
    }
    VerifyingKey512::decode(bytes).ok_or(CryptoError::OperationFailed)
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

fn init_rng(seed: &[u8]) -> SeededRng {
    SeededRng::new(seed)
}

/// Deterministically generates a Falcon keypair from a seed.
pub fn generate_from_seed(seed: &[u8]) -> Result<FalconKeypair, CryptoError> {
    if seed.is_empty() {
        return Err(CryptoError::InvalidKeyLength);
    }

    let mut rng = init_rng(seed);
    let mut public_key = vec![0u8; FALCON_512_PUBLIC_KEY_BYTES];
    let mut secret_key = Zeroizing::new(vec![0u8; FALCON_512_SECRET_KEY_BYTES]);
    let mut keygen = KeyPairGenerator512::default();
    keygen.keygen(
        FALCON_512_LOGN,
        &mut rng,
        secret_key.as_mut_slice(),
        public_key.as_mut_slice(),
    );
    rng.clear();

    Ok(FalconKeypair {
        public_key: FalconPublicKey(public_key),
        secret_key: FalconSecretKey(SecretBytes(std::mem::take(&mut *secret_key))),
    })
}

/// Generates a Falcon keypair from OS randomness.
pub fn generate() -> Result<FalconKeypair, CryptoError> {
    let mut seed = [0u8; 48];
    getrandom(&mut seed).map_err(|_| CryptoError::BackendUnavailable)?;
    let keypair = generate_from_seed(&seed);
    seed.zeroize();
    keypair
}

/// Signs an Atho message under the selected signature domain.
///
/// SECURITY: Domain separation prevents signatures for one protocol context from
/// being replayed as if they were valid in another.
pub fn sign(
    domain: AthoSignatureDomain,
    secret_key: &FalconSecretKey,
    message: &[u8],
) -> Result<FalconSignature, CryptoError> {
    let mut signing_key = decode_signing_key(secret_key.as_bytes())?;
    let mut seed = [0u8; 48];
    getrandom(&mut seed).map_err(|_| CryptoError::BackendUnavailable)?;
    let mut rng = init_rng(&seed);
    seed.zeroize();
    let mut signature = vec![0u8; FALCON_512_SIGNATURE_BYTES];
    signing_key.sign(
        &mut rng,
        &DomainContext(domain.label().as_bytes()),
        &HASH_ID_SHA3_384,
        message,
        &mut signature,
    );
    rng.clear();
    Ok(FalconSignature(signature))
}

/// Verifies an Atho Falcon signature under the selected signature domain.
pub fn verify(
    domain: AthoSignatureDomain,
    public_key: &FalconPublicKey,
    message: &[u8],
    signature: &FalconSignature,
) -> Result<bool, CryptoError> {
    let verifying_key = match decode_verifying_key(public_key.as_bytes()) {
        Ok(key) => key,
        Err(CryptoError::InvalidKeyLength | CryptoError::OperationFailed) => return Ok(false),
        Err(err) => return Err(err),
    };
    if !signature_len_ok(signature.as_bytes().len()) {
        return Ok(false);
    }

    Ok(verifying_key.verify(
        signature.as_bytes(),
        &DomainContext(domain.label().as_bytes()),
        &HASH_ID_SHA3_384,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atho_core::consensus::signatures::{
        transaction_signing_digest, transaction_signing_digest_for_input_indexes,
        AthoSignatureDomain,
    };
    use atho_core::network::Network;
    use atho_core::transaction::{Transaction, TxInput, TxOutput};

    #[derive(Clone, Copy)]
    struct TestRng(u64);

    impl TestRng {
        fn new(seed: u64) -> Self {
            Self(seed)
        }

        fn next_u64(&mut self) -> u64 {
            // Xorshift64* keeps the test deterministic without pulling in an extra dependency.
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn fill_bytes(&mut self, out: &mut [u8]) {
            for chunk in out.chunks_mut(8) {
                let bytes = self.next_u64().to_le_bytes();
                chunk.copy_from_slice(&bytes[..chunk.len()]);
            }
        }

        fn next_len(&mut self, max: usize) -> usize {
            (self.next_u64() as usize % max).saturating_add(1)
        }
    }

    #[test]
    fn falcon512_lengths_are_frozen() {
        assert_eq!(FALCON_512_LOGN, 9);
        assert_eq!(FALCON_512_PUBLIC_KEY_BYTES, 897);
        assert_eq!(FALCON_512_SECRET_KEY_BYTES, 1_281);
        assert_eq!(FALCON_512_SIGNATURE_BYTES, 666);
    }

    #[test]
    fn falcon512_lengths_match_protocol_constants() {
        assert_eq!(
            FALCON_512_PUBLIC_KEY_BYTES,
            atho_core::constants::FALCON_512_PUBLIC_KEY_BYTES
        );
        assert_eq!(
            FALCON_512_SECRET_KEY_BYTES,
            atho_core::constants::FALCON_512_SECRET_KEY_BYTES
        );
        assert_eq!(
            FALCON_512_SIGNATURE_BYTES,
            atho_core::constants::FALCON_512_SIGNATURE_BYTES
        );
    }

    #[test]
    fn falcon_keygen_sign_and_verify_round_trip() {
        let keypair = generate_from_seed(b"atho-falcon-seed").unwrap();
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [1; 48],
                output_index: 0,
                unlocking_script: vec![1, 2, 3],
            }],
            outputs: vec![TxOutput {
                value_atoms: 500,
                locking_script: vec![4, 5],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let digest = transaction_signing_digest(Network::Mainnet, &tx);
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &digest,
        )
        .unwrap();
        assert_eq!(signature.as_bytes().len(), FALCON_512_SIGNATURE_BYTES);
        assert!(verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            &digest,
            &signature,
        )
        .unwrap());
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            b"wrong message",
            &signature,
        )
        .unwrap());
    }

    #[test]
    fn falcon_verify_rejects_wrong_public_key() {
        let signer = generate_from_seed(b"atho-falcon-signer").unwrap();
        let other = generate_from_seed(b"atho-falcon-other").unwrap();
        let message = b"atho signing message";
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &signer.secret_key,
            message,
        )
        .unwrap();
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &other.public_key,
            message,
            &signature,
        )
        .unwrap());
    }

    #[test]
    fn falcon_verify_rejects_malformed_inputs_without_panicking() {
        let keypair = generate_from_seed(b"atho-falcon-malformed-inputs").unwrap();
        let message = b"atho malformed falcon audit message";
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            message,
        )
        .unwrap();

        let truncated_signature =
            FalconSignature(signature.as_bytes()[..FALCON_512_SIGNATURE_BYTES - 1].to_vec());
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            message,
            &truncated_signature,
        )
        .unwrap());

        let mut oversized_signature_bytes = signature.as_bytes().to_vec();
        oversized_signature_bytes.push(0);
        let oversized_signature = FalconSignature(oversized_signature_bytes);
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            message,
            &oversized_signature,
        )
        .unwrap());

        let truncated_public_key = FalconPublicKey(
            keypair.public_key.as_bytes()[..FALCON_512_PUBLIC_KEY_BYTES - 1].to_vec(),
        );
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &truncated_public_key,
            message,
            &signature,
        )
        .unwrap());

        let mut malformed_public_key_bytes = keypair.public_key.as_bytes().to_vec();
        malformed_public_key_bytes[0] ^= 0x80;
        let malformed_public_key = FalconPublicKey(malformed_public_key_bytes);
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &malformed_public_key,
            message,
            &signature,
        )
        .unwrap());

        let empty_signature = FalconSignature(Vec::new());
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            message,
            &empty_signature,
        )
        .unwrap());

        let empty_public_key = FalconPublicKey(Vec::new());
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &empty_public_key,
            message,
            &signature,
        )
        .unwrap());
    }

    #[test]
    fn falcon_length_validation_rejects_wrong_sizes() {
        let err = validate_key_lengths(&[], &[]).unwrap_err();
        assert_eq!(err, CryptoError::InvalidKeyLength);
        assert!(signature_len_ok(FALCON_512_SIGNATURE_BYTES));
        assert!(!signature_len_ok(665));
        assert!(!signature_len_ok(FALCON_512_SIGNATURE_BYTES + 1));
        assert!(public_key_len_ok(FALCON_512_PUBLIC_KEY_BYTES));
        assert!(!public_key_len_ok(896));
        assert!(!public_key_len_ok(FALCON_512_PUBLIC_KEY_BYTES + 1));
    }

    #[test]
    fn falcon_deterministic_keygen_is_stable_and_empty_seed_is_rejected() {
        let a = generate_from_seed(b"atho-falcon-deterministic-seed").unwrap();
        let b = generate_from_seed(b"atho-falcon-deterministic-seed").unwrap();
        let c = generate_from_seed(b"atho-falcon-different-seed").unwrap();

        assert_eq!(a.public_key, b.public_key);
        assert_eq!(a.secret_key.as_bytes(), b.secret_key.as_bytes());
        assert_ne!(a.public_key, c.public_key);
        assert_ne!(a.secret_key.as_bytes(), c.secret_key.as_bytes());
        assert_eq!(
            generate_from_seed(b"").unwrap_err(),
            CryptoError::InvalidKeyLength
        );
    }

    #[test]
    fn falcon_constructor_validation_rejects_wrong_lengths_and_malformed_keys() {
        let keypair = generate_from_seed(b"atho-falcon-ctor-validation").unwrap();

        assert!(FalconPublicKey::from_bytes(keypair.public_key.as_bytes()).is_ok());
        assert!(FalconSecretKey::from_bytes(keypair.secret_key.as_bytes()).is_ok());
        assert!(FalconSignature::from_bytes(&vec![7; FALCON_512_SIGNATURE_BYTES]).is_ok());

        assert_eq!(
            FalconPublicKey::from_bytes(&[]).unwrap_err(),
            CryptoError::InvalidKeyLength
        );
        assert_eq!(
            FalconSecretKey::from_bytes(&[]).unwrap_err(),
            CryptoError::InvalidKeyLength
        );
        assert_eq!(
            FalconSignature::from_bytes(&[]).unwrap_err(),
            CryptoError::InvalidKeyLength
        );

        let mut malformed_public_key = keypair.public_key.as_bytes().to_vec();
        malformed_public_key[0] ^= 0x80;
        assert_eq!(
            FalconPublicKey::from_bytes(&malformed_public_key).unwrap_err(),
            CryptoError::OperationFailed
        );

        let mut malformed_secret_key = keypair.secret_key.as_bytes().to_vec();
        malformed_secret_key[0] ^= 0x80;
        assert_eq!(
            FalconSecretKey::from_bytes(&malformed_secret_key).unwrap_err(),
            CryptoError::OperationFailed
        );
    }

    #[test]
    fn falcon_debug_output_redacts_secret_material() {
        let keypair = generate_from_seed(b"atho-falcon-debug-redaction").unwrap();
        let rendered_secret = format!("{:?}", keypair.secret_key);
        let rendered_keypair = format!("{:?}", keypair);

        assert!(rendered_secret.contains("<redacted>"));
        assert!(!rendered_secret.contains("SecretBytes(["));
        assert!(rendered_keypair.contains("<redacted>"));
        assert!(!rendered_keypair.contains("SecretBytes(["));
    }

    #[test]
    fn falcon_signatures_are_bound_to_network_and_covered_input_set() {
        let keypair = generate_from_seed(b"atho-falcon-network-binding").unwrap();
        let tx = Transaction {
            version: 1,
            inputs: vec![
                TxInput {
                    previous_txid: [1; 48],
                    output_index: 0,
                    unlocking_script: vec![1, 2, 3],
                },
                TxInput {
                    previous_txid: [2; 48],
                    output_index: 1,
                    unlocking_script: vec![4, 5, 6],
                },
            ],
            outputs: vec![TxOutput {
                value_atoms: 500,
                locking_script: vec![7, 8],
            }],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };
        let mainnet_digest = transaction_signing_digest(Network::Mainnet, &tx);
        let testnet_digest = transaction_signing_digest(Network::Testnet, &tx);
        let first_input_digest =
            transaction_signing_digest_for_input_indexes(Network::Mainnet, &tx, &[0]);
        let second_input_digest =
            transaction_signing_digest_for_input_indexes(Network::Mainnet, &tx, &[1]);

        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            &first_input_digest,
        )
        .unwrap();

        assert!(verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            &first_input_digest,
            &signature,
        )
        .unwrap());
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            &second_input_digest,
            &signature,
        )
        .unwrap());
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            &mainnet_digest,
            &signature,
        )
        .unwrap());
        assert!(!verify(
            AthoSignatureDomain::Transaction,
            &keypair.public_key,
            &testnet_digest,
            &signature,
        )
        .unwrap());
    }

    #[test]
    fn falcon_signature_audit_over_10k_random_messages_stays_fixed_size() {
        const SAMPLE_COUNT: usize = 10_000;

        let keypair = generate_from_seed(b"atho-falcon-signature-size-audit").unwrap();
        let mut rng = TestRng::new(0x9E37_79B9_7F4A_7C15);
        let mut message = vec![0u8; 1];
        let mut total_len = 0usize;
        let mut min_len = usize::MAX;
        let mut max_len = 0usize;

        for _ in 0..SAMPLE_COUNT {
            let message_len = rng.next_len(256);
            message.resize(message_len, 0);
            rng.fill_bytes(&mut message);

            let signature = sign(AthoSignatureDomain::TestDev, &keypair.secret_key, &message)
                .expect("falcon signature");
            assert!(verify(
                AthoSignatureDomain::TestDev,
                &keypair.public_key,
                &message,
                &signature,
            )
            .expect("falcon verification"));

            let len = signature.as_bytes().len();
            total_len += len;
            if len < min_len {
                min_len = len;
            }
            if len > max_len {
                max_len = len;
            }
        }

        let avg_len = total_len / SAMPLE_COUNT;
        println!(
            "falcon signature audit: count={SAMPLE_COUNT} min={min_len} max={max_len} avg={avg_len} bytes"
        );
        assert_eq!(min_len, FALCON_512_SIGNATURE_BYTES);
        assert_eq!(max_len, FALCON_512_SIGNATURE_BYTES);
        assert_eq!(avg_len, FALCON_512_SIGNATURE_BYTES);
    }

    #[test]
    fn concurrent_falcon_verification_is_deterministic() {
        let keypair = generate_from_seed(b"atho-falcon-concurrent-verify").unwrap();
        let message = b"atho concurrent falcon verification";
        let signature = sign(
            AthoSignatureDomain::Transaction,
            &keypair.secret_key,
            message,
        )
        .unwrap();
        let wrong_message = b"atho concurrent falcon mutation";

        std::thread::scope(|scope| {
            for _ in 0..8 {
                let public_key = keypair.public_key.clone();
                let signature = signature.clone();
                scope.spawn(move || {
                    for _ in 0..128 {
                        assert!(verify(
                            AthoSignatureDomain::Transaction,
                            &public_key,
                            message,
                            &signature,
                        )
                        .unwrap());
                        assert!(!verify(
                            AthoSignatureDomain::Transaction,
                            &public_key,
                            wrong_message,
                            &signature,
                        )
                        .unwrap());
                        assert!(!verify(
                            AthoSignatureDomain::Block,
                            &public_key,
                            message,
                            &signature,
                        )
                        .unwrap());
                    }
                });
            }
        });
    }
}

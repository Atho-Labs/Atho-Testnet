// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Encrypted wallet `.datafile` persistence.
use super::{PersistedWalletState, Wallet};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use atho_core::network::Network;
use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, WALLET_INVALID_HEADER, WALLET_INVALID_PASSWORD, WALLET_IO,
    WALLET_RANDOMNESS_FAILURE, WALLET_SERIALIZATION, WALLET_UNSUPPORTED_ENCRYPTION_MODE,
    WALLET_UNSUPPORTED_VERSION,
};
use getrandom::getrandom;
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use std::fs::{self, File};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use thiserror::Error;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"ATHODF01";
const VERSION: u16 = 1;
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const PASSWORD_ITERATIONS: u32 = 600_000;
const AAD_PREFIX: &[u8] = b"atho-wallet-datafile";
const DEFAULT_PASSWORD_SCHEME: &str = "atho-wallet-password-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletEncryptionMode {
    Plaintext = 0,
    PasswordAes256Gcm = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletDatafileMetadata {
    pub network: Network,
    pub encryption_mode: WalletEncryptionMode,
}

#[derive(Debug, Error)]
pub enum WalletDatafileError {
    #[error("io error")]
    Io(#[from] std::io::Error),
    #[error("serialization error")]
    Encode(#[from] bincode::Error),
    #[error("invalid datafile header")]
    InvalidHeader,
    #[error("unsupported wallet datafile version")]
    UnsupportedVersion,
    #[error("unsupported encryption mode")]
    UnsupportedEncryptionMode,
    #[error("randomness failure")]
    RandomnessFailure,
    #[error("password rejected or data corrupted")]
    InvalidPassword,
}

impl AthoErrorMeta for WalletDatafileError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::Io(_) => &WALLET_IO,
            Self::Encode(_) => &WALLET_SERIALIZATION,
            Self::InvalidHeader => &WALLET_INVALID_HEADER,
            Self::UnsupportedVersion => &WALLET_UNSUPPORTED_VERSION,
            Self::UnsupportedEncryptionMode => &WALLET_UNSUPPORTED_ENCRYPTION_MODE,
            Self::RandomnessFailure => &WALLET_RANDOMNESS_FAILURE,
            Self::InvalidPassword => &WALLET_INVALID_PASSWORD,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-wallet::datafile"
    }

    fn safe_details(&self) -> Option<String> {
        match self {
            Self::Io(error) => Some(error.to_string()),
            Self::Encode(error) => Some(error.to_string()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WalletDataFile {
    pub network: Network,
    pub encryption_mode: WalletEncryptionMode,
    pub salt: [u8; SALT_BYTES],
    pub nonce: [u8; NONCE_BYTES],
    pub ciphertext: Vec<u8>,
}

impl WalletDataFile {
    pub fn save(wallet: &Wallet, password: &str, path: &Path) -> Result<(), WalletDatafileError> {
        save_impl(wallet, password, path, PASSWORD_ITERATIONS)
    }

    #[doc(hidden)]
    pub fn save_with_iterations(
        wallet: &Wallet,
        password: &str,
        path: &Path,
        iterations: u32,
    ) -> Result<(), WalletDatafileError> {
        save_impl(wallet, password, path, iterations)
    }

    pub fn load(path: &Path, password: &str) -> Result<Wallet, WalletDatafileError> {
        load_impl(path, password, PASSWORD_ITERATIONS)
    }

    #[doc(hidden)]
    pub fn load_with_iterations(
        path: &Path,
        password: &str,
        iterations: u32,
    ) -> Result<Wallet, WalletDatafileError> {
        load_impl(path, password, iterations)
    }

    pub fn load_with_progress<F>(
        path: &Path,
        password: &str,
        progress: F,
    ) -> Result<Wallet, WalletDatafileError>
    where
        F: FnMut(usize, usize),
    {
        load_impl_with_progress(path, password, PASSWORD_ITERATIONS, progress)
    }

    pub fn inspect(path: &Path) -> Result<WalletDatafileMetadata, WalletDatafileError> {
        let bytes = fs::read(path)?;
        let file = WalletDataFile::from_bytes(&bytes)?;
        Ok(WalletDatafileMetadata {
            network: file.network,
            encryption_mode: file.encryption_mode,
        })
    }

    fn to_bytes(&self) -> Result<Vec<u8>, WalletDatafileError> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.push(self.network as u8);
        out.push(self.encryption_mode as u8);
        out.extend_from_slice(&self.salt);
        out.extend_from_slice(&self.nonce);
        let len =
            u32::try_from(self.ciphertext.len()).map_err(|_| WalletDatafileError::InvalidHeader)?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&self.ciphertext);
        Ok(out)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, WalletDatafileError> {
        let min_len = MAGIC.len() + 2 + 1 + 1 + SALT_BYTES + NONCE_BYTES + 4;
        if bytes.len() < min_len || &bytes[..MAGIC.len()] != MAGIC {
            return Err(WalletDatafileError::InvalidHeader);
        }

        let version = u16::from_le_bytes(bytes[8..10].try_into().expect("slice length"));
        if version != VERSION {
            return Err(WalletDatafileError::UnsupportedVersion);
        }

        let network = match bytes[10] {
            0 => Network::Mainnet,
            1 => Network::Testnet,
            2 => Network::Regnet,
            _ => return Err(WalletDatafileError::InvalidHeader),
        };

        let encryption_mode = match bytes[11] {
            0 => WalletEncryptionMode::Plaintext,
            1 => WalletEncryptionMode::PasswordAes256Gcm,
            _ => return Err(WalletDatafileError::InvalidHeader),
        };

        let mut salt = [0u8; SALT_BYTES];
        salt.copy_from_slice(&bytes[12..12 + SALT_BYTES]);

        let mut nonce = [0u8; NONCE_BYTES];
        nonce.copy_from_slice(&bytes[12 + SALT_BYTES..12 + SALT_BYTES + NONCE_BYTES]);

        let payload_len_start = 12 + SALT_BYTES + NONCE_BYTES;
        let payload_len = u32::from_le_bytes(
            bytes[payload_len_start..payload_len_start + 4]
                .try_into()
                .expect("slice length"),
        ) as usize;
        let payload_start = payload_len_start + 4;
        let payload_end = payload_start + payload_len;
        if payload_end > bytes.len() {
            return Err(WalletDatafileError::InvalidHeader);
        }

        Ok(Self {
            network,
            encryption_mode,
            salt,
            nonce,
            ciphertext: bytes[payload_start..payload_end].to_vec(),
        })
    }
}

fn save_impl(
    wallet: &Wallet,
    password: &str,
    path: &Path,
    iterations: u32,
) -> Result<(), WalletDatafileError> {
    let state = wallet.capture_state();
    // An empty passphrase is functionally "no encryption". Persist that case honestly and avoid
    // paying a large PBKDF2 startup cost for wallets the user intentionally left unencrypted.
    let mut salt = [0u8; SALT_BYTES];
    let mut nonce = [0u8; NONCE_BYTES];
    let (encryption_mode, ciphertext) = if password.is_empty() {
        (WalletEncryptionMode::Plaintext, bincode::serialize(&state)?)
    } else {
        getrandom(&mut salt).map_err(|_| WalletDatafileError::RandomnessFailure)?;
        getrandom(&mut nonce).map_err(|_| WalletDatafileError::RandomnessFailure)?;
        (
            WalletEncryptionMode::PasswordAes256Gcm,
            encrypt_state(password, &salt, &nonce, &state, iterations)?,
        )
    };
    let file = WalletDataFile {
        network: wallet.network,
        encryption_mode,
        salt,
        nonce,
        ciphertext,
    };
    atomic_write(path, &file.to_bytes()?)?;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), WalletDatafileError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("wallet.datafile");
    let tmp_path = path.with_file_name(format!("{file_name}.tmp"));
    {
        let mut file = File::create(&tmp_path)?;
        restrict_owner_only_permissions(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    restrict_owner_only_permissions(path)?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn restrict_owner_only_permissions(path: &Path) -> Result<(), WalletDatafileError> {
    #[cfg(unix)]
    {
        let permissions = std::fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn load_impl(path: &Path, password: &str, iterations: u32) -> Result<Wallet, WalletDatafileError> {
    load_impl_with_progress(path, password, iterations, |_, _| {})
}

fn load_impl_with_progress<F>(
    path: &Path,
    password: &str,
    iterations: u32,
    progress: F,
) -> Result<Wallet, WalletDatafileError>
where
    F: FnMut(usize, usize),
{
    let bytes = fs::read(path)?;
    let file = WalletDataFile::from_bytes(&bytes)?;
    let state = match file.encryption_mode {
        WalletEncryptionMode::Plaintext => bincode::deserialize(&file.ciphertext)?,
        WalletEncryptionMode::PasswordAes256Gcm => decrypt_state(
            password,
            &file.salt,
            &file.nonce,
            &file.ciphertext,
            iterations,
        )?,
    };
    Ok(Wallet::from_state_with_progress(
        file.network,
        None,
        state,
        progress,
    ))
}

fn encrypt_state(
    password: &str,
    salt: &[u8; SALT_BYTES],
    nonce: &[u8; NONCE_BYTES],
    state: &PersistedWalletState,
    iterations: u32,
) -> Result<Vec<u8>, WalletDatafileError> {
    let mut key = [0u8; 32];
    let password = Zeroizing::new(password.as_bytes().to_vec());
    pbkdf2_hmac::<Sha256>(&password, salt, iterations, &mut key);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("key length");
    let payload = bincode::serialize(state)?;
    cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &payload,
                aad: &build_aad(),
            },
        )
        .map_err(|_| WalletDatafileError::InvalidPassword)
}

fn decrypt_state(
    password: &str,
    salt: &[u8; SALT_BYTES],
    nonce: &[u8; NONCE_BYTES],
    ciphertext: &[u8],
    iterations: u32,
) -> Result<PersistedWalletState, WalletDatafileError> {
    let mut key = [0u8; 32];
    let password = Zeroizing::new(password.as_bytes().to_vec());
    pbkdf2_hmac::<Sha256>(&password, salt, iterations, &mut key);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("key length");
    let payload = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &build_aad(),
            },
        )
        .map_err(|_| WalletDatafileError::InvalidPassword)?;
    Ok(bincode::deserialize(&payload)?)
}

fn build_aad() -> Vec<u8> {
    let mut aad = Vec::with_capacity(AAD_PREFIX.len() + DEFAULT_PASSWORD_SCHEME.len() + 2);
    aad.extend_from_slice(AAD_PREFIX);
    aad.extend_from_slice(DEFAULT_PASSWORD_SCHEME.as_bytes());
    aad.extend_from_slice(&VERSION.to_le_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mnemonic::{MnemonicLength, MnemonicPhrase};
    use crate::wallet::WALLET_DATAFILE_NAME;
    use atho_errors::AthoErrorMeta;
    use std::env;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    const TEST_ITERATIONS: u32 = 10_000;

    fn wallet() -> Wallet {
        let mnemonic = MnemonicPhrase::from_entropy(&[0u8; 32], MnemonicLength::Words24).unwrap();
        let mut wallet = Wallet::from_mnemonic(mnemonic, "", Network::Mainnet);
        wallet.checkout_receive_address();
        wallet.checkout_change_address();
        wallet
    }

    #[test]
    fn datafile_name_is_stable() {
        assert_eq!(WALLET_DATAFILE_NAME, ".datafile");
    }

    #[test]
    fn save_and_load_wallet_datafile_round_trips_state() {
        let dir = env::temp_dir();
        let path = dir.join("atho-wallet-roundtrip.datafile");
        let wallet = wallet();
        save_impl(&wallet, "password", &path, TEST_ITERATIONS).unwrap();
        let loaded = load_impl(&path, "password", TEST_ITERATIONS).unwrap();
        assert_eq!(loaded.network, Network::Mainnet);
        assert_eq!(loaded.snapshot.receive_count, wallet.snapshot.receive_count);
        assert_eq!(loaded.snapshot.change_count, wallet.snapshot.change_count);
        assert_eq!(loaded.address_book.len(), wallet.address_book.len());
        assert_eq!(loaded.mnemonic_sentence(), wallet.mnemonic_sentence());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let dir = env::temp_dir();
        let path = dir.join("atho-wallet-wrong-password.datafile");
        let wallet = wallet();
        save_impl(&wallet, "password", &path, TEST_ITERATIONS).unwrap();
        let err = load_impl(&path, "wrong", TEST_ITERATIONS).unwrap_err();
        assert!(matches!(err, WalletDatafileError::InvalidPassword));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn empty_password_wallet_uses_plaintext_mode() {
        let dir = env::temp_dir();
        let path = dir.join("atho-wallet-plaintext.datafile");
        let wallet = wallet();
        save_impl(&wallet, "", &path, TEST_ITERATIONS).unwrap();

        let metadata = WalletDataFile::inspect(&path).unwrap();
        assert_eq!(metadata.network, Network::Mainnet);
        assert_eq!(metadata.encryption_mode, WalletEncryptionMode::Plaintext);

        let loaded = load_impl(&path, "", TEST_ITERATIONS).unwrap();
        assert_eq!(loaded.mnemonic_sentence(), wallet.mnemonic_sentence());
        assert_eq!(loaded.snapshot, wallet.snapshot);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn plaintext_wallet_load_reports_timing() {
        let dir = env::temp_dir();
        let path = dir.join("atho-wallet-plaintext-timing.datafile");
        let wallet = wallet();

        let save_started = Instant::now();
        save_impl(&wallet, "", &path, TEST_ITERATIONS).unwrap();
        let save_elapsed = save_started.elapsed();

        let load_started = Instant::now();
        let loaded = load_impl(&path, "", TEST_ITERATIONS).unwrap();
        let load_elapsed = load_started.elapsed();

        eprintln!(
            "wallet_datafile_timings plaintext_save_ms={} plaintext_load_ms={}",
            save_elapsed.as_millis(),
            load_elapsed.as_millis()
        );

        assert_eq!(loaded.snapshot, wallet.snapshot);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn wallet_errors_stay_sanitized_and_coded() {
        let error = WalletDatafileError::InvalidPassword.to_atho_error();
        assert_eq!(error.code().as_str(), "ATHO-WALLET-011");
        assert!(!error.to_string().contains("hunter2"));
    }

    #[cfg(unix)]
    #[test]
    fn wallet_datafile_permissions_are_owner_only() {
        let path = env::temp_dir().join(format!(
            "atho-wallet-perms-{}-{}.datafile",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let wallet = wallet();
        save_impl(&wallet, "password", &path, TEST_ITERATIONS).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_file(&path);
    }
}

use atho_errors::{
    AthoErrorDescriptor, AthoErrorMeta, WALLET_INVALID_ENTROPY_LENGTH,
    WALLET_INVALID_MNEMONIC_CHECKSUM, WALLET_INVALID_MNEMONIC_WORD,
    WALLET_INVALID_MNEMONIC_WORD_COUNT,
};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use thiserror::Error;
use zeroize::Zeroize;

pub mod wordlist;

pub const MNEMONIC_SCHEME: &str = "atho-mnemonic-v1";
pub const MNEMONIC_PBKDF2_ITERATIONS: u32 = 600_000;
pub const DEFAULT_MNEMONIC_WORD_COUNT: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MnemonicLength {
    Words12,
    Words24,
    Words48,
}

impl MnemonicLength {
    pub fn word_count(self) -> usize {
        match self {
            Self::Words12 => 12,
            Self::Words24 => 24,
            Self::Words48 => 48,
        }
    }

    pub fn entropy_bytes(self) -> usize {
        match self {
            Self::Words12 => 16,
            Self::Words24 => 32,
            Self::Words48 => 64,
        }
    }

    pub fn from_word_count(words: usize) -> Option<Self> {
        match words {
            12 => Some(Self::Words12),
            24 => Some(Self::Words24),
            48 => Some(Self::Words48),
            _ => None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MnemonicError {
    #[error("invalid mnemonic word count")]
    InvalidWordCount,
    #[error("invalid mnemonic entropy length")]
    InvalidEntropyLength,
    #[error("invalid mnemonic word")]
    InvalidWord,
    #[error("mnemonic checksum mismatch")]
    ChecksumMismatch,
}

impl AthoErrorMeta for MnemonicError {
    fn descriptor(&self) -> &'static AthoErrorDescriptor {
        match self {
            Self::InvalidWordCount => &WALLET_INVALID_MNEMONIC_WORD_COUNT,
            Self::InvalidEntropyLength => &WALLET_INVALID_ENTROPY_LENGTH,
            Self::InvalidWord => &WALLET_INVALID_MNEMONIC_WORD,
            Self::ChecksumMismatch => &WALLET_INVALID_MNEMONIC_CHECKSUM,
        }
    }

    fn source_module(&self) -> &'static str {
        "atho-wallet::mnemonic"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[zeroize(drop)]
pub struct MnemonicPhrase {
    words: Vec<String>,
}

impl MnemonicPhrase {
    pub fn from_entropy(entropy: &[u8], length: MnemonicLength) -> Result<Self, MnemonicError> {
        if entropy.len() != length.entropy_bytes() {
            return Err(MnemonicError::InvalidEntropyLength);
        }
        let words = encode_entropy(entropy, length)?;
        Ok(Self { words })
    }

    pub fn parse(input: &str) -> Result<Self, MnemonicError> {
        let words: Vec<String> = input
            .split_whitespace()
            .map(|word| word.trim().to_lowercase())
            .filter(|word| !word.is_empty())
            .collect();
        let length =
            MnemonicLength::from_word_count(words.len()).ok_or(MnemonicError::InvalidWordCount)?;
        validate_words(&words)?;
        validate_checksum(&words, length)?;
        Ok(Self { words })
    }

    pub fn words(&self) -> &[String] {
        &self.words
    }

    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    pub fn as_sentence(&self) -> String {
        self.words.join(" ")
    }

    pub fn root_seed(&self, passphrase: &str) -> [u8; 64] {
        let mut seed = [0u8; 64];
        let salt = format!("{}{}", MNEMONIC_SCHEME, passphrase);
        pbkdf2_hmac::<Sha512>(
            self.as_sentence().as_bytes(),
            salt.as_bytes(),
            MNEMONIC_PBKDF2_ITERATIONS,
            &mut seed,
        );
        seed
    }
}

fn validate_words(words: &[String]) -> Result<(), MnemonicError> {
    for word in words {
        if wordlist::word_to_index(word).is_none() {
            return Err(MnemonicError::InvalidWord);
        }
    }
    Ok(())
}

fn encode_entropy(entropy: &[u8], length: MnemonicLength) -> Result<Vec<String>, MnemonicError> {
    let checksum_bits = entropy.len() * 8 / 32;
    let digest = Sha256::digest(entropy);
    let total_bits = entropy.len() * 8 + checksum_bits;
    let word_count = total_bits / 11;
    let mut words = Vec::with_capacity(word_count);
    for idx in 0..word_count {
        let bit_offset = idx * 11;
        let word_index = read_bits(entropy, &digest, bit_offset, 11);
        let word = wordlist::index_to_word(word_index).ok_or(MnemonicError::InvalidWord)?;
        words.push(word.to_string());
    }
    if word_count != length.word_count() {
        return Err(MnemonicError::InvalidWordCount);
    }
    Ok(words)
}

fn validate_checksum(words: &[String], length: MnemonicLength) -> Result<(), MnemonicError> {
    let entropy_bits = length.entropy_bytes() * 8;
    let checksum_bits = entropy_bits / 32;
    let mut entropy = vec![0u8; length.entropy_bytes()];
    for (i, word) in words.iter().enumerate() {
        let idx = wordlist::word_to_index(word).ok_or(MnemonicError::InvalidWord)? as usize;
        write_bits(&mut entropy, i * 11, 11, idx as u32);
    }
    let digest = Sha256::digest(&entropy);
    for bit in 0..checksum_bits {
        let expected = bit_from_bytes(&digest, bit);
        let actual = bit_from_bytes_from_words(words, entropy_bits + bit);
        if expected != actual {
            return Err(MnemonicError::ChecksumMismatch);
        }
    }
    Ok(())
}

fn read_bits(entropy: &[u8], checksum: &[u8], start_bit: usize, width: usize) -> u16 {
    let mut value = 0u16;
    for offset in 0..width {
        value <<= 1;
        let bit = if start_bit + offset < entropy.len() * 8 {
            bit_from_bytes(entropy, start_bit + offset)
        } else {
            bit_from_bytes(checksum, start_bit + offset - entropy.len() * 8)
        };
        value |= bit as u16;
    }
    value
}

fn write_bits(target: &mut [u8], start_bit: usize, width: usize, value: u32) {
    for offset in 0..width {
        let bit_index = width - 1 - offset;
        let bit = ((value >> bit_index) & 1) as u8;
        let pos = start_bit + offset;
        let byte = pos / 8;
        if byte >= target.len() {
            continue;
        }
        let shift = 7 - (pos % 8);
        target[byte] |= bit << shift;
    }
}

fn bit_from_bytes(bytes: &[u8], bit_index: usize) -> u8 {
    let byte = bytes[bit_index / 8];
    let shift = 7 - (bit_index % 8);
    (byte >> shift) & 1
}

fn bit_from_bytes_from_words(words: &[String], bit_index: usize) -> u8 {
    let mut current = 0usize;
    for word in words {
        let idx = wordlist::word_to_index(word).expect("validated") as usize;
        if bit_index < current + 11 {
            let offset = bit_index - current;
            return ((idx >> (10 - offset)) & 1) as u8;
        }
        current += 11;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_lengths_are_exposed() {
        assert_eq!(MnemonicLength::Words12.word_count(), 12);
        assert_eq!(MnemonicLength::Words24.word_count(), 24);
        assert_eq!(MnemonicLength::Words48.word_count(), 48);
        assert_eq!(DEFAULT_MNEMONIC_WORD_COUNT, 24);
    }

    #[test]
    fn mnemonic_round_trips_for_allowed_lengths() {
        let m12 = MnemonicPhrase::from_entropy(&[0u8; 16], MnemonicLength::Words12).unwrap();
        let m24 = MnemonicPhrase::from_entropy(&[1u8; 32], MnemonicLength::Words24).unwrap();
        let m48 = MnemonicPhrase::from_entropy(&[2u8; 64], MnemonicLength::Words48).unwrap();

        assert_eq!(m12.word_count(), 12);
        assert_eq!(m24.word_count(), 24);
        assert_eq!(m48.word_count(), 48);

        let round_trip = MnemonicPhrase::parse(&m24.as_sentence()).unwrap();
        assert_eq!(round_trip.word_count(), 24);
        assert_eq!(round_trip.as_sentence(), m24.as_sentence());
    }

    #[test]
    fn mnemonic_seed_is_stable() {
        let phrase = MnemonicPhrase::from_entropy(&[0u8; 32], MnemonicLength::Words24).unwrap();
        let a = phrase.root_seed("");
        let b = phrase.root_seed("");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn mnemonic_parse_rejects_bad_checksum() {
        let phrase = MnemonicPhrase::from_entropy(&[0u8; 32], MnemonicLength::Words24).unwrap();
        let mut words = phrase.words().to_vec();
        words[0] = if words[0] == wordlist::index_to_word(0).unwrap() {
            wordlist::index_to_word(1).unwrap().to_string()
        } else {
            wordlist::index_to_word(0).unwrap().to_string()
        };
        let altered = words.join(" ");
        assert!(matches!(
            MnemonicPhrase::parse(&altered),
            Err(MnemonicError::ChecksumMismatch)
        ));
    }
}

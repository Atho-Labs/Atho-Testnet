use crate::{constants::*, crypto::hash::sha3_256, network::Network};

fn base56_value(c: char) -> Option<u8> {
    BASE56_ALPHABET
        .chars()
        .position(|ch| ch == c)
        .map(|v| v as u8)
}

fn base56_encode_bytes(data: &[u8]) -> String {
    let base = BASE56_ALPHABET.len() as u32;
    if data.is_empty() {
        return BASE56_ALPHABET.chars().next().unwrap_or('2').to_string();
    }

    let mut digits = vec![0u8];
    for &byte in data {
        let mut carry = byte as u32;
        for digit in &mut digits {
            let value = (*digit as u32) * 256 + carry;
            *digit = (value % base) as u8;
            carry = value / base;
        }
        while carry > 0 {
            digits.push((carry % base) as u8);
            carry /= base;
        }
    }

    let mut out = String::with_capacity(digits.len());
    for digit in digits.iter().rev() {
        out.push(BASE56_ALPHABET.as_bytes()[*digit as usize] as char);
    }
    out
}

fn base56_encode_fixed_bytes(data: &[u8], width: usize) -> String {
    let encoded = base56_encode_bytes(data);
    if encoded.len() >= width {
        encoded
    } else {
        let mut out = String::with_capacity(width);
        for _ in 0..(width - encoded.len()) {
            out.push(BASE56_ALPHABET.chars().next().unwrap_or('2'));
        }
        out.push_str(&encoded);
        out
    }
}

fn base56_decode_to_bytes(
    s: &str,
    expected_len: usize,
) -> Result<Vec<u8>, crate::error::AddressError> {
    let base = BASE56_ALPHABET.len() as u32;
    if s.is_empty() {
        return Err(crate::error::AddressError::InvalidAlphabet);
    }

    let mut bytes = vec![0u8];
    for ch in s.chars() {
        let value = base56_value(ch).ok_or(crate::error::AddressError::InvalidAlphabet)? as u32;
        let mut carry = value;
        for byte in bytes.iter_mut().rev() {
            let acc = (*byte as u32) * base + carry;
            *byte = (acc & 0xFF) as u8;
            carry = acc >> 8;
        }
        while carry > 0 {
            bytes.insert(0, (carry & 0xFF) as u8);
            carry >>= 8;
        }
    }

    if bytes.len() > expected_len {
        return Err(crate::error::AddressError::InvalidChecksum);
    }
    if bytes.len() < expected_len {
        let mut padded = vec![0u8; expected_len - bytes.len()];
        padded.extend_from_slice(&bytes);
        return Ok(padded);
    }
    Ok(bytes)
}

pub fn address_checksum(prefix: char, body: &str) -> [u8; ADDRESS_CHECKSUM_BYTES] {
    let mut data = Vec::with_capacity(1 + body.len());
    data.push(prefix as u8);
    data.extend_from_slice(body.as_bytes());
    let digest = sha3_256(&data);
    digest[..ADDRESS_CHECKSUM_BYTES]
        .try_into()
        .expect("fixed length")
}

pub fn role_digest_from_pubkey(
    network: Network,
    public_key: &[u8],
    role: &str,
) -> [u8; ADDRESS_DIGEST_BYTES] {
    let mut preimage = Vec::with_capacity(
        1 + ADDRESS_ROLE_DOMAIN.len() + 1 + network.domain_tag().len() + 1 + public_key.len(),
    );
    preimage.extend_from_slice(role.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(network.domain_tag().as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(public_key);
    sha3_256(&preimage)
}

pub fn public_key_digest(network: Network, public_key: &[u8]) -> [u8; ADDRESS_DIGEST_BYTES] {
    role_digest_from_pubkey(network, public_key, ADDRESS_ROLE_DOMAIN)
}

pub fn hashed_public_key_from_digest(
    network: Network,
    digest: &[u8; ADDRESS_DIGEST_BYTES],
) -> String {
    format!("{}{}", network.internal_hpk_prefix(), hex::encode(digest))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressParts {
    pub network: Network,
    pub visible_prefix: char,
    pub base56_address: String,
    pub hashed_public_key: String,
    pub payment_digest: [u8; ADDRESS_DIGEST_BYTES],
    pub checksum: [u8; ADDRESS_CHECKSUM_BYTES],
}

pub fn address_parts_from_public_key(network: Network, public_key: &[u8]) -> AddressParts {
    let payment_digest = public_key_digest(network, public_key);
    let visible_prefix = network.visible_prefix();
    let body = base56_encode_bytes(&payment_digest);
    let checksum = address_checksum(visible_prefix, &body);
    let checksum_body = base56_encode_fixed_bytes(&checksum, ADDRESS_CHECKSUM_BASE56_CHARS);
    let mut base56_address = String::with_capacity(1 + body.len() + checksum_body.len());
    base56_address.push(visible_prefix);
    base56_address.push_str(&body);
    base56_address.push_str(&checksum_body);
    AddressParts {
        network,
        visible_prefix,
        base56_address,
        hashed_public_key: hashed_public_key_from_digest(network, &payment_digest),
        payment_digest,
        checksum,
    }
}

pub fn internal_hpk_bytes(network: Network, internal_hpk: &str) -> Option<Vec<u8>> {
    let prefix = network.internal_hpk_prefix();
    let body = internal_hpk.strip_prefix(prefix)?;
    if body.len() != crate::constants::SHA3_384_HASH_HEX_CHARS {
        return None;
    }
    hex::decode(body).ok()
}

pub fn address_from_public_key(network: Network, public_key: &[u8]) -> String {
    address_parts_from_public_key(network, public_key).base56_address
}

pub fn is_base56_char(c: char) -> bool {
    BASE56_ALPHABET.contains(c)
}

pub fn is_visible_prefix(c: char) -> bool {
    matches!(c, 'A' | 'T')
}

pub fn encode_base56_address(network: Network, digest: &[u8; ADDRESS_DIGEST_BYTES]) -> String {
    let prefix = network.visible_prefix();
    let body = base56_encode_bytes(digest);
    let checksum = address_checksum(prefix, &body);
    let checksum_body = base56_encode_fixed_bytes(&checksum, ADDRESS_CHECKSUM_BASE56_CHARS);
    let mut out = String::with_capacity(1 + body.len() + checksum_body.len());
    out.push(prefix);
    out.push_str(&body);
    out.push_str(&checksum_body);
    out
}

pub fn decode_base56_address(
    address: &str,
) -> Result<([u8; ADDRESS_DIGEST_BYTES], Network), crate::error::AddressError> {
    if !address
        .chars()
        .next()
        .map(is_visible_prefix)
        .unwrap_or(false)
    {
        return Err(crate::error::AddressError::InvalidPrefix);
    }
    if address.len() <= 1 + ADDRESS_CHECKSUM_BASE56_CHARS {
        return Err(crate::error::AddressError::InvalidChecksum);
    }
    let prefix = address
        .chars()
        .next()
        .ok_or(crate::error::AddressError::InvalidPrefix)?;
    let body = &address[1..address.len() - ADDRESS_CHECKSUM_BASE56_CHARS];
    let checksum_body = &address[address.len() - ADDRESS_CHECKSUM_BASE56_CHARS..];
    let network = match prefix {
        'A' => Network::Mainnet,
        'T' => Network::Testnet,
        _ => return Err(crate::error::AddressError::InvalidPrefix),
    };
    let expected_checksum = address_checksum(prefix, body);
    let expected_checksum_body =
        base56_encode_fixed_bytes(&expected_checksum, ADDRESS_CHECKSUM_BASE56_CHARS);
    if checksum_body != expected_checksum_body {
        return Err(crate::error::AddressError::InvalidChecksum);
    }
    let decoded = base56_decode_to_bytes(body, ADDRESS_DIGEST_BYTES)?;
    let mut out = [0u8; ADDRESS_DIGEST_BYTES];
    out.copy_from_slice(&decoded);
    Ok((out, network))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_fixed_size() {
        let sum = address_checksum('A', "123456");
        assert_eq!(sum.len(), 4);
    }

    #[test]
    fn visible_prefix_and_alphabet_rules_hold() {
        assert!(is_visible_prefix('A'));
        assert!(is_visible_prefix('T'));
        assert!(is_base56_char('2'));
        assert!(!is_base56_char('0'));
    }

    #[test]
    fn base56_address_round_trips() {
        let digest = [7u8; ADDRESS_DIGEST_BYTES];
        let address = encode_base56_address(Network::Mainnet, &digest);
        let (decoded, network) = decode_base56_address(&address).unwrap();
        assert_eq!(decoded, digest);
        assert_eq!(network, Network::Mainnet);
    }

    #[test]
    fn address_parts_include_internal_hpk_and_base56() {
        let parts = address_parts_from_public_key(Network::Testnet, &[42u8; 32]);
        assert_eq!(parts.network, Network::Testnet);
        assert!(parts.base56_address.starts_with('T'));
        assert!(parts
            .hashed_public_key
            .starts_with(Network::Testnet.internal_hpk_prefix()));
        assert_eq!(
            parts.hashed_public_key,
            hashed_public_key_from_digest(Network::Testnet, &parts.payment_digest)
        );
        assert_eq!(
            parts.hashed_public_key.len(),
            Network::Testnet.internal_hpk_prefix().len() + ADDRESS_DIGEST_BYTES * 2
        );
    }

    #[test]
    fn internal_hpk_decodes_to_raw_bytes() {
        let encoded = format!(
            "{}{}",
            Network::Mainnet.internal_hpk_prefix(),
            "ab".repeat(crate::constants::SHA3_384_HASH_BITS / 8)
        );
        let decoded = internal_hpk_bytes(Network::Mainnet, &encoded).unwrap();
        assert_eq!(decoded.len(), crate::constants::SHA3_384_HASH_BITS / 8);
        assert!(decoded.iter().all(|byte| *byte == 0xab));
    }
}

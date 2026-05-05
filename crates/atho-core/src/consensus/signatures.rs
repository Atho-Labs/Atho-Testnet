use crate::block::Block;
use crate::crypto::hash::sha3_384;
use crate::genesis::genesis_hash;
use crate::network::Network;
use crate::transaction::Transaction;

pub const ATHO_SIGNATURE_RULES_VERSION: u32 = 1;

pub const ATHO_TX_SIGN_V1: &str = "ATHO_TX_SIGN_V1";
pub const ATHO_BLOCK_SIG_V1: &str = "ATHO_BLOCK_SIG_V1";
pub const ATHO_WALLET_LOCAL_SIG_V1: &str = "ATHO_WALLET_LOCAL_SIG_V1";
pub const ATHO_PACKAGE_SIG_V1: &str = "ATHO_PACKAGE_SIG_V1";
pub const ATHO_TEST_DEV_SIG_V1: &str = "ATHO_TEST_DEV_SIG_V1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AthoSignatureDomain {
    Transaction,
    Block,
    WalletLocal,
    Package,
    TestDev,
}

impl AthoSignatureDomain {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Transaction => ATHO_TX_SIGN_V1,
            Self::Block => ATHO_BLOCK_SIG_V1,
            Self::WalletLocal => ATHO_WALLET_LOCAL_SIG_V1,
            Self::Package => ATHO_PACKAGE_SIG_V1,
            Self::TestDev => ATHO_TEST_DEV_SIG_V1,
        }
    }
}

/// Canonical transaction prehash used for Atho Falcon signatures.
///
/// This is the exact message prehash passed to Falcon-512 RS under the
/// `ATHO_TX_SIGN_V1` domain:
///
/// - canonical source: `Transaction::base_bytes()` plus the covered input set
/// - hash function: `SHA3-384`
/// - output size: 48 bytes
pub fn transaction_signing_digest(network: Network, tx: &Transaction) -> [u8; 48] {
    let mut preimage = Vec::with_capacity(ATHO_TX_SIGN_V1.len() + 1 + 48 + 48);
    preimage.extend_from_slice(ATHO_TX_SIGN_V1.as_bytes());
    preimage.push(network.consensus_id());
    preimage.extend_from_slice(&genesis_hash(network));
    preimage.extend_from_slice(&tx.signing_digest());
    sha3_384(&preimage)
}

/// Canonical grouped-signer prehash used when one transaction spends multiple
/// wallet address groups.
pub fn transaction_signing_digest_for_input_indexes(
    network: Network,
    tx: &Transaction,
    input_indexes: &[u32],
) -> [u8; 48] {
    let mut preimage = Vec::with_capacity(ATHO_TX_SIGN_V1.len() + 1 + 48 + 48);
    preimage.extend_from_slice(ATHO_TX_SIGN_V1.as_bytes());
    preimage.push(network.consensus_id());
    preimage.extend_from_slice(&genesis_hash(network));
    preimage.extend_from_slice(&tx.signing_digest_for_input_indexes(input_indexes));
    sha3_384(&preimage)
}

/// Canonical block prehash reserved for Atho block-signature use.
///
/// Block signatures are not currently active, but the domain label and
/// prehash rules are frozen here so future use cannot drift silently.
pub fn block_signing_digest(block: &Block) -> [u8; 48] {
    sha3_384(&block.header.canonical_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::Network;
    use crate::transaction::{Transaction, TxInput, TxOutput};

    #[test]
    fn signature_domains_are_frozen() {
        assert_eq!(ATHO_SIGNATURE_RULES_VERSION, 1);
        assert_eq!(AthoSignatureDomain::Transaction.label(), ATHO_TX_SIGN_V1);
        assert_eq!(AthoSignatureDomain::Block.label(), ATHO_BLOCK_SIG_V1);
        assert_eq!(
            AthoSignatureDomain::WalletLocal.label(),
            ATHO_WALLET_LOCAL_SIG_V1
        );
        assert_eq!(AthoSignatureDomain::Package.label(), ATHO_PACKAGE_SIG_V1);
        assert_eq!(AthoSignatureDomain::TestDev.label(), ATHO_TEST_DEV_SIG_V1);
        assert_eq!(ATHO_TX_SIGN_V1, "ATHO_TX_SIGN_V1");
        assert_eq!(ATHO_BLOCK_SIG_V1, "ATHO_BLOCK_SIG_V1");
        assert_eq!(ATHO_WALLET_LOCAL_SIG_V1, "ATHO_WALLET_LOCAL_SIG_V1");
        assert_eq!(ATHO_PACKAGE_SIG_V1, "ATHO_PACKAGE_SIG_V1");
        assert_eq!(ATHO_TEST_DEV_SIG_V1, "ATHO_TEST_DEV_SIG_V1");
    }

    #[test]
    fn transaction_prehash_is_canonical() {
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

        assert_ne!(
            transaction_signing_digest(Network::Mainnet, &tx),
            tx.signing_digest()
        );
        assert_eq!(Network::Mainnet.consensus_id(), 1);
    }

    #[test]
    fn transaction_prehash_is_network_scoped() {
        let tx = Transaction {
            version: 1,
            inputs: vec![TxInput {
                previous_txid: [9; 48],
                output_index: 2,
                unlocking_script: vec![7; 32],
            }],
            outputs: vec![TxOutput {
                value_atoms: 1_000,
                locking_script: vec![8; 32],
            }],
            lock_time: 11,
            witness: vec![],
            tx_pow_nonce: 0,
            tx_pow_bits: 0,
        };

        assert_ne!(
            transaction_signing_digest(Network::Mainnet, &tx),
            transaction_signing_digest(Network::Testnet, &tx)
        );
        assert_ne!(
            transaction_signing_digest_for_input_indexes(Network::Mainnet, &tx, &[0]),
            transaction_signing_digest_for_input_indexes(Network::Testnet, &tx, &[0])
        );
    }
}

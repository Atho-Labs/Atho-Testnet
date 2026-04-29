# Transactions

## Purpose

Atho transactions move value by spending existing UTXOs and creating new ones.

The design is intentionally narrow:

- integer-only amounts
- one canonical transaction encoding
- one canonical signing digest
- explicit witness ownership checks

## Transaction Structure

Implemented in:

- `crates/atho-core/src/transaction.rs`

Core fields:

- `version: u16`
- `inputs: Vec<TxInput>`
- `outputs: Vec<TxOutput>`
- `lock_time: u32`
- `witness: Vec<u8>`

Input fields:

- `previous_txid: [u8; 48]`
- `output_index: u32`
- `unlocking_script: Vec<u8>`

Output fields:

- `value_atoms: u64`
- `locking_script: Vec<u8>`

Why:

- the structure stays close to Bitcoin’s UTXO spend model while adapting to Atho’s 48-byte hash space and witness commitment model

## Canonical Encoding

Transactions expose several byte layouts:

- `base_bytes()`
- `full_bytes()`
- `compact_bytes()`
- `canonical_bytes()`

Consensus-critical identity uses the canonical path defined in code. The signing digest is derived from the base form.

Why:

- different operational views are useful for size accounting and transport
- consensus still needs one canonical source of truth

## Transaction Identity

The canonical transaction identifier is a 48-byte hash.

The code computes:

- `txid()` from canonical transaction bytes
- `signing_digest()` from the canonical pre-signing representation

Implemented in:

- `crates/atho-core/src/transaction.rs`
- `crates/atho-core/src/consensus/signatures.rs`

Why:

- txid and signing digest must be explicit and separate to avoid silent rule drift

## Signature Model

Atho uses:

- Falcon-512 signatures
- SHA3-384 prehashing
- explicit domain separation

The active transaction domain label is:

- `ATHO_TX_SIG_V1`

Implemented in:

- `crates/atho-core/src/consensus/signatures.rs`
- `crates/atho-storage/src/validation.rs`
- `crates/atho-crypto/src/falcon.rs`

Why:

- domain separation reduces ambiguity across transaction, wallet-local, package, and future block-signature contexts

## Witness Model

Witnesses are stored separately from the base spend description and include:

- signature bytes
- public key bytes
- per-input reference material

The validation layer derives:

- short signature references
- witness commitment references

Implemented in:

- `crates/atho-core/src/transaction.rs`
- `crates/atho-storage/src/validation.rs`

Why:

- witness commitments allow pruning-safe validation references without overloading the spend script path

## Validation Rules

Current transaction validation checks include:

- supported transaction version
- non-empty outputs
- maximum raw size
- maximum vsize
- non-zero output values
- duplicate input rejection
- minimum fee floor
- witness presence and shape
- witness reference correctness
- public key and signature size checks
- Falcon signature verification
- referenced UTXO existence and ownership
- maturity and confirmation rules

Implemented in:

- `crates/atho-storage/src/validation.rs`

Why:

- transaction validity must be completely backend-owned and deterministic

## Fee Policy

Current fee floor:

- `1 atom / vbyte`

Constants:

- `MIN_TX_FEE_PER_VBYTE_ATOMS`
- `MIN_TX_FEE_ATOMS`

Why:

- a simple explicit fee floor is easier to reason about while the network policy layer is still early-stage

## Lifecycle Placement

Transaction flow today:

1. wallet builds a candidate spend
2. wallet signs the canonical digest
3. node validates and admits to mempool
4. miner selects by feerate
5. block acceptance reruns validation
6. confirmed outputs enter the UTXO set

## Current Limitations

- no richer relay policy surface yet
- no completed orphan transaction pool
- wallet activity history is still reconstructed indirectly instead of from a canonical history API

## Related Documentation

- [Addresses and Keys](addresses-and-keys.md)
- [Mining and Mempool](../node-runtime/mining-and-mempool.md)
- [Wallet Model](../wallet/wallet-model.md)

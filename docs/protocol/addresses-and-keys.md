# Addresses and Keys

## Purpose

Atho addresses give users a human-facing payment identifier while preserving a deterministic mapping back to key-derived payment digests.

## Address Model

Implemented in:

- `crates/atho-core/src/address.rs`

The address system uses:

- a 32-byte payment digest
- a visible network prefix (`A`, `T`, or `R`)
- Base56 text encoding
- a 4-byte checksum rendered as 6 Base56 characters

Why:

- the visible prefix makes network mistakes easier to catch
- the checksum makes transcription errors detectable
- the alphabet avoids visually ambiguous characters

## Hashed Public Key Representation

Atho also uses an internal hashed-public-key text form:

- mainnet prefix: `ATHO`
- testnet/regnet prefix: `ATHT`

This representation is useful for scripts and wallet ownership comparisons.

Why:

- a human-facing payment address and a backend-friendly internal key identifier serve different operational needs

## Network-Specific Identity

Address derivation depends on:

- the network domain tag
- the role label
- the public key bytes

The current role-domain label is:

- `ATHO_ADDR_V1`

Why:

- address identity must be network-local and role-local to prevent cross-context reuse ambiguities

## Wallet Derivation

The wallet derives Falcon keypairs from:

- a 32-byte wallet seed
- network domain tag
- account number
- address kind (`Receive` or `Change`)
- index

Implemented in:

- `crates/atho-wallet/src/hd.rs`
- `crates/atho-wallet/src/wallet.rs`

Why:

- receive and change paths should be distinct
- deterministic derivation keeps restore behavior reproducible

## Address Validation

Base56 decoding checks:

- visible prefix
- minimum length
- alphabet membership
- checksum match
- exact decoded digest width

Why:

- incorrect addresses should fail fast before wallet or node logic tries to interpret them further

## Current Limitations

- no address-book indexing inside node storage beyond the base record plumbing
- no advanced label synchronization across GUI and backend yet

## Related Documentation

- [Wallet Model](../wallet/wallet-model.md)
- [Transactions](transactions.md)
- [Cryptography](../crypto/cryptography.md)

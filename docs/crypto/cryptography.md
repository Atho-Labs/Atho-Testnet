# Cryptography

## Purpose

Atho’s cryptographic layer is intentionally narrow and explicit.

It covers:

- hashing
- transaction signing and verification
- wallet-local secret handling
- domain separation

## Hash Functions

Current hash choices:

- `SHA3-256` for address and smaller domain-separated digests
- `SHA3-384` for block hashes, signing prehashes, and 48-byte protocol identifiers

Implemented in:

- `crates/atho-core/src/crypto/hash.rs`

Why:

- Atho uses SHA3 as part of its protocol identity while keeping digest selection explicit by use case

## Signature Scheme

Current active signature scheme:

- Falcon-512

Relevant sizes:

| Item | Size |
| --- | ---: |
| Public key | `897 bytes` |
| Secret key | `1,281 bytes` |
| Signature | `666 bytes` |

Implemented in:

- `crates/atho-crypto/src/falcon.rs`
- vendored `fn-dsa` crates under `Falcon 512 rs/`

Why:

- Atho is explicitly exploring a post-quantum signature model instead of reusing secp256k1

Tradeoff:

- larger signatures affect transaction size and relay policy
- the cryptographic vendor tree needs careful path and documentation handling

## Domain Separation

Current frozen labels:

- `ATHO_TX_SIGN_V1`
- `ATHO_BLOCK_SIG_V1`
- `ATHO_WALLET_LOCAL_SIG_V1`
- `ATHO_PACKAGE_SIG_V1`
- `ATHO_TEST_DEV_SIG_V1`

Implemented in:

- `crates/atho-core/src/consensus/signatures.rs`

Why:

- the same signature primitive should not silently serve unrelated message contexts under one label

## Secret Handling

The wallet and crypto layers use:

- explicit secret types
- zeroization where required

Implemented in:

- `crates/atho-crypto/src/secret.rs`
- `crates/atho-wallet/src/hd.rs`
- `crates/atho-wallet/src/wallet/datafile.rs`

Why:

- in-memory hygiene matters even when consensus does not directly depend on it

## Wallet Datafile Encryption

The wallet datafile uses:

- AES-256-GCM
- PBKDF2-HMAC-SHA256
- a versioned binary envelope

Why:

- wallet-at-rest protection is an operational requirement independent of consensus design

## Current Limitations

- no signature aggregation or compact witness scheme
- no protocol-level block signature activation
- no external cryptographic verification service; validation is local only

## Related Documentation

- [Addresses and Keys](../protocol/addresses-and-keys.md)
- [Transactions](../protocol/transactions.md)
- [Crypto Migration Report](migration-report.md)

# Atho Wallet Code Guide

## Scope

Wallet code mainly lives in:

- `crates/atho-wallet/src/wallet.rs`
- `crates/atho-wallet/src/hd.rs`
- `crates/atho-wallet/src/keypool.rs`
- `crates/atho-wallet/src/mnemonic/`
- `crates/atho-wallet/src/wallet/datafile.rs`
- `crates/atho-qt/src/app/pages/send.rs`
- `crates/atho-qt/src/app/pages/receive.rs`

## Secret Handling

Wallet code must never:

- log mnemonics
- log seeds
- log private keys
- log raw secret-key bytes
- serialize secrets into error strings

Use explicit `WALLET SECURITY` comments anywhere code touches:

- mnemonic phrases
- seed material
- secret keys
- encrypted wallet payloads

## Derivation Rule

Address and key derivation must stay deterministic for:

- network
- address role
- derivation path

## UX Rule

Wallet UI should make dangerous actions explicit:

- exporting secrets
- copying recovery material
- generating addresses
- building spend transactions

## Policy Rule

Wallet transaction construction should follow standard relay policy, including fee floors and the 1,000-atom minimum output rule, unless a dedicated expert flow says otherwise.

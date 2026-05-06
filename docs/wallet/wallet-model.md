# Wallet Model

## Purpose

The Atho wallet owns seed material, deterministic address derivation, keypool state, encrypted local persistence, and wallet-local user data.

It does not own consensus.

## Core Model

Implemented in:

- `crates/atho-wallet/src/wallet.rs`
- `crates/atho-wallet/src/hd.rs`
- `crates/atho-wallet/src/keypool.rs`
- `crates/atho-wallet/src/address_book.rs`
- `crates/atho-wallet/src/snapshot.rs`

The client supports multiple HD wallets. Each wallet has its own wallet ID, user-facing wallet name, network association, metadata, derived addresses, derivation indexes, UTXO view, transaction history/cache, and address book.

The wallet includes:

- network binding
- unique wallet ID
- wallet name
- optional mnemonic
- HD wallet state
- keypool
- per-wallet address book
- wallet snapshot counters
- restore gap limit
- created and updated timestamps

## Wallet Creation

Wallet creation requires:

- wallet name
- mnemonic word count selection

Supported mnemonic word counts are:

- 12 words
- 24 words
- 48 words

The default is 24 words. Wallet names are user-facing metadata used to organize multiple wallets; they are not the file identity and must not be used as the only storage key.

Creation must not overwrite existing wallets. The client persists wallet metadata and registers the wallet before making the new wallet current.

## Mnemonic Import

Mnemonic import works directly and does not require creating another wallet first.

Supported import forms:

- one-line phrase
- newline-separated phrase
- numbered phrase such as `01. word` or `1) word`
- extra whitespace between words
- 12-word, 24-word, and 48-word phrases

The client normalizes whitespace and numbering, validates word count, validates spelling against the wordlist, validates checksum or entropy mapping where applicable, derives the wallet root, persists the wallet, and scans or rescans derived addresses when backend data is available.

Never share a seed phrase. Do not store mnemonic material insecurely, and do not log mnemonics, seeds, private keys, wallet passwords, or derived private keys.

## Deterministic Derivation

Receive and change paths are separate.

Current derivation metadata:

- account: `0`
- kind: `Receive` or `Change`
- index: monotonically increasing per kind
- next receive and change indexes are wallet-local

Why:

- separate receive/change tracks keep output ownership and address reuse easier to reason about

## Keypool

Current target sizes:

- receive pool: up to `5,000`
- change pool: up to `5,000`

Why:

- the GUI can request fresh addresses without blocking on expensive key-generation work every time

## Restore Behavior

Current restore gap limit:

- `1,000`

Why:

- Atho needs a deterministic discovery boundary for wallet rehydration without scanning infinitely

## Wallet Switching

Users switch wallets from the File menu through **Open / Switch Wallet**. The File menu is the wallet lifecycle entry point and should include or reference:

- New Wallet
- Import Wallet
- Open / Switch Wallet
- Backup Wallet
- Lock Wallet, if supported
- Rename Wallet, if supported

Switching wallets changes the active wallet context. The client must unload stale send state, selected UTXOs, selected addresses, address book entries, receive addresses, transaction history, and balance data before loading the selected wallet. A wallet must never spend another wallet's UTXOs or sign with another wallet's keys.

## Per-Wallet Address Book

Address books are wallet-local. Each wallet owns its own saved labels and addresses.

Address book entries include:

- ID
- label
- address
- network
- optional notes
- created and updated timestamps
- optional last-used timestamp

The address book stores public addresses and labels only. It does not store private keys. Addresses are validated before saving, and wrong-network addresses are rejected.

## Datafile Persistence

Current wallet datafile properties:

- binary envelope
- versioned header
- AES-256-GCM encryption
- PBKDF2-HMAC-SHA256 password derivation
- `.datafile` filename convention

Why:

- wallet-at-rest encryption is an operational minimum even in a local-first architecture

## Address And Balance Ownership

Each wallet tracks:

- derived addresses
- receive and change counts
- locally known UTXO ownership through backend scanning

Current limitation:

- canonical activity history is not yet sourced from a dedicated node history API
- the Qt client still reconstructs history from exported ledger views

That is one of the main remaining product-quality gaps.

## Related Documentation

- [Addresses and Keys](../protocol/addresses-and-keys.md)
- [Qt Client](../gui-client/qt-client.md)
- [Current Production Status](../production-readiness/current-status.md)

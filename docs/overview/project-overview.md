# Project Overview

## What Atho Is

Atho is a Bitcoin-style public UTXO payment stack implemented in Rust.

It includes:

- a consensus core
- a full validating node
- durable chainstate storage
- a multi-wallet system with HD derivation and encrypted datafiles
- a local RPC interface
- a thin desktop client
- a developing peer-to-peer network layer

## Mission

Atho aims to be a small, explicit, and production-minded blockchain stack where every important rule is local, deterministic, and auditable.

The project biases toward:

- boring architecture
- explicit constants
- deterministic validation
- one canonical rule path
- compact modules
- low abstraction overhead
- backend-owned truth

## What Problem Atho Is Solving

Atho is not trying to be a general smart-contract platform or a privacy-first Layer 1 launch.

Its narrower goal is:

- a direct payment chain
- a public UTXO ledger
- independent local verification
- post-quantum signature readiness
- a software stack that feels closer to Bitcoin Core discipline than to application-platform sprawl

## Major Design Choices

### Monetary policy

Atho uses 12 decimal places:

- `DECIMALS = 12`
- `ATOMS_PER_ATHO = 1_000_000_000_000`
- `1 ATHO = 1,000,000,000,000 atoms`

There is no fixed max supply cap. Atho uses proof-of-work with a permanent tail reward so miners retain a predictable long-term security budget while user fees can remain low.

Current schedule:

- target block time: 75 seconds
- initial reward: 6.25 ATHO
- halving interval: 1,680,000 blocks
- permanent tail reward: 0.78125 ATHO per block

The official display ladder is:

- 1 ATHO = 1,000 mATHO
- 1 mATHO = 1,000 μATHO
- 1 μATHO = 1,000 nATHO
- 1 nATHO = 1,000 atoms
- 1 atom = smallest network unit

Consensus stores integer atoms only. Display units are UI-only.

### Transaction policy

Normal transactions use low atom-denominated fees plus wallet transaction proof-of-work as a spam deterrent.

Current policy:

- required fee: `max(500 atoms, tx_vbytes * 1 atom)`
- minimum normal output: 1,000 atoms
- maximum standard outputs: 64
- normal transactions require SHA3-256 transaction PoW
- coinbase transactions do not require wallet transaction PoW

Wallets sign first, then generate the transaction send proof over the signed transaction without PoW fields. Nodes can verify that proof before expensive signature checks when possible.

### Bitcoin-style UTXO core

Chosen because:

- UTXO state transitions are compact and explicit
- replay, rollback, and reorg logic are easier to reason about than account-style global mutation
- wallet ownership and spendability are inspectable through discrete outputs

Tradeoff:

- richer stateful application logic is intentionally not a Layer 1 goal

### Rust implementation

Chosen because:

- memory safety matters in consensus and networking code
- the workspace benefits from explicit ownership and type boundaries
- performance-sensitive code can still stay close to the metal

Tradeoff:

- vendored or specialized cryptographic integrations require careful path and build hygiene

### Thin desktop client

Chosen because:

- the node should remain the authority for chainstate, validation, and mining
- UI state should reflect backend truth instead of embedding a second blockchain implementation
- restart and synchronization behavior is easier to harden when the GUI is a client

Tradeoff:

- wallet history and client UX are dependent on stable backend interfaces

### Multi-wallet client model

The desktop client supports multiple HD wallets. Wallet creation requires a wallet name and a 12-word, 24-word, or 48-word mnemonic selection; 24 words is the default. Mnemonic import works directly and supports one-line, newline-separated, numbered, and extra-whitespace phrases.

Users switch wallets from **File -> Open / Switch Wallet**. Each wallet keeps its own metadata, addresses, UTXO state, transaction history/cache, derivation indexes, and per-wallet address book.

### Mainnet and testnet separation

Mainnet and testnet are strictly separated. Transactions, signatures, transaction PoW preimages, addresses, peers, storage, UTXOs, mempool state, and blocks are network-scoped.

Mainnet has no faucet and no testnet difficulty stall reset. Recoverable storage repair is shared across networks but remains network-scoped: damaged local chainstate/index data is quarantined under the active network and rebuilt from that network's configuration. Testnet may reset during development and may reset difficulty to minimum after more than 10 minutes without a block. Testnet ATHO is distributed manually by the Atho founders or development team.

### Explicit versioning and activation scaffolding

Chosen because:

- silent consensus drift is unacceptable
- upgrades need one clear height-gated routing mechanism
- storage versioning and ruleset versioning should be inspectable in code

Tradeoff:

- the current stack includes placeholders for future rulesets before those rules exist

## Non-Goals At Current Stage

- public mainnet launch readiness
- fully decentralized peer mesh operation
- snapshot sync deployment
- complex Layer 1 scripting
- private transaction schemes

## Code Map

Primary implementation locations:

- `crates/atho-core/src/`
- `crates/atho-storage/src/`
- `crates/atho-node/src/`
- `crates/atho-wallet/src/`
- `crates/atho-p2p/src/`
- `crates/atho-rpc/src/`
- `crates/atho-qt/src/`

Related documentation:

- [System Architecture](../architecture/system-architecture.md)
- [Blocks and Consensus](../protocol/blocks-and-consensus.md)
- [Current Production Status](../production-readiness/current-status.md)

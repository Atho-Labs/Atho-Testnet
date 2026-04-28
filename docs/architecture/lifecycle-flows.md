# Lifecycle Flows

This document summarizes the major end-to-end flows that define how Atho behaves at runtime.

## Transaction Lifecycle

```mermaid
flowchart TD
    A[Wallet derives receive/change addresses] --> B[User creates spend]
    B --> C[Wallet assembles tx inputs, outputs, fee, witness]
    C --> D[Wallet signs canonical tx digest]
    D --> E[Node receives RpcRequest::SubmitTransaction]
    E --> F[Mempool validation against current UTXO set]
    F -->|valid| G[Transaction enters mempool]
    F -->|invalid| H[Rejected with deterministic error]
    G --> I[Miner builds candidate block]
    I --> J[Block validation reruns tx checks]
    J --> K[Accepted block removes tx from mempool]
    K --> L[UTXO set and wallet-visible balances update]
```

Why it is shaped this way:

- transaction creation stays wallet-owned
- transaction acceptance stays node-owned
- transaction validity is checked again at block-accept time
- mempool policy and block consensus share the same underlying validation logic

## Block Lifecycle

```mermaid
flowchart TD
    A[Node builds candidate block template] --> B[Coinbase reward and fee totals computed]
    B --> C[Merkle root and witness root computed]
    C --> D[Header assembled with target and parent]
    D --> E[Miner searches nonce]
    E -->|meets target| F[Node submits block]
    F --> G[Canonical block validation]
    G -->|valid| H[UTXO apply + atomic persistence]
    G -->|invalid| I[Reject without final-state mutation]
    H --> J[Tip advances]
```

Why it is shaped this way:

- miners do not bypass validation
- block acceptance is separate from block construction
- persistence happens only after validation passes

## Restart And Reload Lifecycle

```mermaid
flowchart TD
    A[Process starts] --> B[Open LMDB environment]
    B --> C[Check schema version]
    C --> D[Load chainstate snapshot]
    D --> E[Load persisted block history]
    E -->|valid| F[Rehydrate chainstate and runtime status]
    E -->|corrupt or incomplete| G[Quarantine local state]
    G --> H[Rebuild fresh state from genesis]
```

Why it is shaped this way:

- fail-closed is safer than silent partial recovery
- quarantine preserves forensic context without trusting damaged state

## Wallet And Qt Lifecycle

```mermaid
flowchart TD
    A[Qt starts] --> B[Connect to local or RPC backend]
    B --> C[Open or create wallet]
    C --> D[Derive address and scan UTXOs]
    D --> E[Display balances and history]
    E --> F[Submit send or mine request]
    F --> G[Backend state changes]
    G --> H[Qt polls status and UTXOs]
    H --> I[Refresh balances, mempool count, and tip height]
```

Why it is shaped this way:

- the GUI remains stateless relative to consensus truth
- chain tip and balances are refreshed from backend reality

## Reorg Lifecycle

```mermaid
flowchart TD
    A[Candidate branch arrives] --> B[Validate branch sequence]
    B --> C[Find fork point in retained history]
    C --> D[Compare accumulated chainwork]
    D -->|less work| E[Keep current chain]
    D -->|more work| F[Disconnect current suffix]
    F --> G[Apply new suffix]
    G -->|success| H[Rebuild mempool against new tip]
    G -->|failure| I[Restore previous suffix]
```

Why it is shaped this way:

- invalid competing history must not contaminate final state
- rollback and reapply need to be deterministic

## Related Documentation

- [Transactions](../protocol/transactions.md)
- [Blocks and Consensus](../protocol/blocks-and-consensus.md)
- [Wallet Model](../wallet/wallet-model.md)
- [Qt Client](../gui-client/qt-client.md)

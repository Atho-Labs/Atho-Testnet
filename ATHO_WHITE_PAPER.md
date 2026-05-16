# Atho White Paper
## Post-Quantum Proof-of-Work Digital Money

**Ghost Genull**  
**Contact:** labs@atho.io  
**The Platinum Standard of the Quantum Age**

This document is grounded in the current Atho codebase in this repository. Where older planning notes, prompts, or public-facing summaries differ from the implemented consensus rules, the code is treated as authoritative.

## Code-Grounded Policy Note

Before drafting this white paper, the current repository constants, consensus modules, API exposure, and network policy files were reviewed. The live implementation currently enforces:

- a **100-second** target block time
- **6 standard confirmations** for ordinary UTXO spendability policy across networks
- **100-block** coinbase maturity across networks
- a **5 ATHO** initial block subsidy
- a **1,260,000-block** halving interval
- a **0.625 ATHO** perpetual tail reward beginning after the third halving
- no finite maximum ATHO supply cap in the current code
- fixed **founder-hash metadata** fields (`SHA3-384` and `SHA3-512`) in canonical header serialization

The white paper below reflects those live rules rather than older planning language or superseded public descriptions.

## 1. Abstract

Atho is a payment-focused Proof-of-Work blockchain designed around deterministic validation, a UTXO accounting model, and post-quantum-aware transaction authorization. The network uses SHA3-384 for block hashing and Falcon-512 signatures for transaction witnesses. Its architecture favors auditable base-layer rules, predictable validation, compact ownership semantics, and operational simplicity over broad virtual-machine complexity.

In the current code, Atho uses 12 decimal places, defines 1 ATHO as 1,000,000,000,000 atoms, targets 100-second block production, and applies a halving-based emission schedule that transitions into a perpetual 0.625 ATHO tail reward. Full nodes validate block structure, proof of work, monetary rules, ownership binding, witness correctness, and UTXO spendability independently. The result is a simple Layer 1 built for secure payments, miner-enforced settlement, and long-term reviewability.

### 1.1. Modern Cryptography and Atho's Post-Quantum Design

Bitcoin and many other established cryptocurrency systems rely on elliptic-curve signature systems such as ECDSA or Schnorr variants. Those systems are efficient and widely deployed today, but long-term cryptographic planning must also account for the theoretical effect of sufficiently capable quantum systems running algorithms such as Shor's algorithm. That does not mean current elliptic-curve systems are presently broken in practice. It does mean long-horizon protocol design benefits from explicit post-quantum planning.

Atho moves transaction authorization toward that planning model by using Falcon-512 signatures. Falcon-512 is a lattice-based signature scheme standardized in the post-quantum cryptography era and is used in Atho for witness verification on transaction spends. Atho also uses SHA3-384 as its proof-of-work and hashing foundation, drawing on the NIST-standardized SHA-3 family and its sponge-based design. Together, those choices preserve the operational clarity of a Proof-of-Work chain while reducing dependence on legacy elliptic-curve assumptions in transaction authorization.

## 2. Introduction

Digital money is most useful when it is understandable, independently verifiable, and difficult to compromise. Over time, many blockchain systems expanded into large execution environments with broad smart-contract surfaces, bridge dependencies, wrapped-asset risk, and monetary models that are hard for operators to audit directly. Those designs can be useful in some contexts, but they also widen the failure surface.

Atho takes a different path. It is built as a payment-oriented Layer 1 with deterministic validation, post-quantum-aware signatures, simple UTXO ownership semantics, and miner-secured Proof-of-Work ordering. The design goal is not to maximize complexity. The design goal is to provide a chain that users, miners, wallet developers, exchanges, and auditors can reason about without ambiguity.

That philosophy is visible throughout the codebase. Ownership is bound directly to canonical lock digests. Transaction witnesses are validated strictly. Legacy lock forms are rejected. Monetary emission is defined by small, explicit consensus constants rather than mutable off-chain policy. The result is a codebase that aims to stay readable, predictable, and difficult to misinterpret.

## 3. Core Design Principles

### 3.1. Security First

Atho prioritizes strict validation order, explicit ownership binding, fail-closed witness verification, and deterministic block acceptance.

### 3.2. Post-Quantum Awareness

Atho uses Falcon-512 for transaction authorization to reduce dependence on elliptic-curve signatures in its ownership model.

### 3.3. Proof-of-Work Fairness

Blocks are mined through SHA3-384 Proof-of-Work. Miners compete by expending verifiable computational effort rather than relying on privileged issuance.

### 3.4. Simplicity Over Complexity

The base layer is focused on payments, settlement, and chain validation rather than general-purpose virtual-machine execution.

### 3.5. Payment Precision

Twelve decimal places give the network fine-grained fee and payment precision without requiring floating-point accounting.

### 3.6. Long-Term Auditability

Consensus constants, reward transitions, ownership checks, and witness rules are written so they can be reimplemented and independently verified.

### 3.7. Deterministic Operations

Nodes accept or reject the same canonical transaction and block bytes under the same consensus conditions. APIs, wallets, miners, and P2P paths are expected to feed into those same rules rather than bypass them.

## 4. Technical Overview

| Parameter | Current Code Value | Plain-Language Meaning |
|---|---|---|
| Consensus | Proof of Work | Blocks are ordered through verifiable computational work. |
| Proof-of-Work hash | SHA3-384 | Miners search for a 384-bit digest below the active target. |
| Signature scheme | Falcon-512 | Transaction witnesses prove ownership with a lattice-based signature scheme. |
| Decimal places | 12 | ATHO supports very fine-grained payment precision. |
| Base unit | atom | The smallest indivisible accounting unit in consensus. |
| Atoms per ATHO | 1,000,000,000,000 | One ATHO equals one trillion atoms. |
| Target block time | 100 seconds | The current code targets 36 blocks per hour and 315,360 blocks per year. |
| Initial block reward | 5 ATHO | The starting subsidy per block before halvings. |
| Halving interval | 1,260,000 blocks | Subsidy halves at fixed height intervals. |
| Tail reward | 0.625 ATHO | Emission floors at a fixed tail instead of halving below that point. |
| Max supply | Not currently capped in code | Current consensus does not enforce a finite ATHO supply ceiling. |
| Coinbase maturity | 100 blocks | Coinbase outputs cannot be spent immediately after creation. |
| Standard confirmations | 6 across all networks | Ordinary spendability policy uses the same threshold across network modes. |
| Validation model | Deterministic full-node validation | Nodes independently validate block structure, witnesses, value flow, and chainwork. |
| Ownership model | Canonical 32-byte lock digest + witness public key binding | UTXOs are spent only when the provided witness key matches the stored canonical lock. |
| Header metadata | Fixed founder hashes in canonical header bytes | Current headers commit to consensus founder metadata fields. |

SHA3-384 produces a 384-bit digest and gives Atho a larger hashing margin than shorter digest sizes while preserving deterministic, machine-verifiable output for mining and block identification. Falcon-512 is used for transaction authorization rather than proof of work. Users prove spend authority by producing a valid witness signature over the canonical signing message for a transaction input. Nodes parse the public key and signature, rebuild the signing context, and verify the result before accepting the spend.

Twelve decimals matter because payment systems benefit from divisibility. Small relay fees, wallet consolidation, merchant pricing, and long-term asset appreciation are all easier to support when the base layer can express very small values without changing the ledger's economic unit.

### 4.1. System Flow

```text
+------------------+      +----------------------+      +----------------------+
| Wallet / Peer /  | ---> | Strict byte decode   | ---> | Canonical structure  |
| API raw payload  |      | and size limits      |      | and network checks   |
+------------------+      +----------------------+      +----------------------+
                                                                |
                                                                v
                                         +--------------------------------------+
                                         | UTXO lookup, ownership binding,      |
                                         | Falcon witness verification, fees,   |
                                         | and spendability enforcement         |
                                         +--------------------------------------+
                                                                |
                                                                v
                                      +------------------------+-----------------------+
                                      | Mempool admission, mining inclusion, block     |
                                      | validation, and final state commit             |
                                      +------------------------------------------------+
```

## 5. Atho Monetary Policy

The current Atho codebase enforces a deterministic emission schedule, but it does **not** presently enforce a finite maximum supply cap. That distinction matters. Earlier planning text described a 78,000,000 ATHO cap, but the active consensus parameters do not implement that policy. Instead, the code uses:

- an initial block reward of **5 ATHO**
- a halving interval of **1,260,000 blocks**
- a minimum tail reward of **0.625 ATHO**
- a perpetual tail phase beginning at block **3,780,000**

Every full node is expected to reject a block that mints more subsidy than the schedule allows at a given height. Every full node is also expected to reject invalid value flow, incorrect fees, malformed coinbase amounts, and ownership violations. That is how the network enforces its monetary behavior in practice: through deterministic block acceptance, not through trust in wallets or miners.

### 5.1. Why the Current Monetary Policy Still Matters

Even without a finite cap in the current code, the monetary policy still needs to be auditable for several reasons:

- miners need a stable reward schedule
- wallet and exchange software need predictable atom accounting
- node operators need deterministic rejection conditions
- community reviewers need to understand how issuance behaves over time
- future monetary changes, if any, require an explicit consensus transition rather than informal expectation

### 5.2. Why 12 Decimal Places Matter

High divisibility lets the system support small relay fees, consolidation transactions, merchant pricing, and long-tail value transfers without inflating the displayed unit count artificially. Because all consensus accounting uses integer atoms, Atho achieves that precision without floating-point ambiguity.

## 6. Emission Model and 150-Year Supply Projection

The current code implements a **halving-based emission schedule with a perpetual tail reward**.

In plain language:

1. The chain starts at **5 ATHO** per block.
2. The block subsidy halves every **1,260,000 blocks**.
3. Once the halved reward would drop below **0.625 ATHO**, the schedule floors there instead.
4. Under the current constants, tail emission begins at height **3,780,000**.
5. Because the current code does not enforce a finite max-supply cap, the supply keeps increasing during tail emission.

The reward function implemented in code is:

```text
reward(height) = max(5 / 2^floor(height / 1,260,000), 0.625) ATHO
```

The current constants define a **100-second** target block time and **315,360** blocks per year. Tail emission begins after about **11.99 years** under those constants.

Table 1  
*Atho 150-Year Monetary Supply Projection Under the Current Emission Policy*

| Year | Estimated Block Height | Block Reward (ATHO) | Annual Emission (ATHO) | Cumulative Supply (ATHO) | % of Enforced Max Supply | Remaining to Enforced Max Supply |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 315,360 | 5 | 1,576,800 | 1,576,800 | N/A | N/A |
| 2 | 630,720 | 5 | 1,576,800 | 3,153,600 | N/A | N/A |
| 3 | 946,080 | 5 | 1,576,800 | 4,730,400 | N/A | N/A |
| 4 | 1,261,440 | 2.5 | 1,573,200 | 6,303,600 | N/A | N/A |
| 5 | 1,576,800 | 2.5 | 788,400 | 7,092,000 | N/A | N/A |
| 10 | 3,153,600 | 1.25 | 394,200 | 10,242,000 | N/A | N/A |
| 15 | 4,730,400 | 0.625 | 197,100 | 11,619,000 | N/A | N/A |
| 20 | 6,307,200 | 0.625 | 197,100 | 12,604,500 | N/A | N/A |
| 25 | 7,884,000 | 0.625 | 197,100 | 13,590,000 | N/A | N/A |
| 30 | 9,460,800 | 0.625 | 197,100 | 14,575,500 | N/A | N/A |
| 40 | 12,614,400 | 0.625 | 197,100 | 16,546,500 | N/A | N/A |
| 50 | 15,768,000 | 0.625 | 197,100 | 18,517,500 | N/A | N/A |
| 75 | 23,652,000 | 0.625 | 197,100 | 23,445,000 | N/A | N/A |
| 100 | 31,536,000 | 0.625 | 197,100 | 28,372,500 | N/A | N/A |
| 125 | 39,420,000 | 0.625 | 197,100 | 33,300,000 | N/A | N/A |
| 150 | 47,304,000 | 0.625 | 197,100 | 38,227,500 | N/A | N/A |

Note. Projection uses the active code constants: 100-second blocks, 315,360 blocks per year, a 5 ATHO initial subsidy, 1,260,000-block halvings, and a perpetual 0.625 ATHO tail reward. Current consensus does not enforce a finite maximum supply cap, so the last two columns are not applicable. Annual emission figures exclude transaction fees because fees redistribute existing ATHO rather than mint new ATHO.

### 6.1. What the Table Means

The schedule front-loads the first three eras, then transitions into a constant tail reward. That means the chain's long-term emission becomes linear after tail activation rather than asymptotically approaching a fixed cap. Under the current code, cumulative issuance reaches **38,227,500 ATHO after 150 years**, but issuance does not terminate at that point.

## 7. Consensus and Validation

Atho uses Proof of Work for block production and deterministic node validation for block acceptance. Miners search for valid SHA3-384 block hashes under the current network target. Full nodes then independently validate the result. A block with valid proof of work is still rejected if it breaks any other consensus rule.

Nodes validate:

- block header structure and network identity
- canonical block and transaction serialization
- proof-of-work target satisfaction
- timestamp and difficulty rules
- coinbase position and subsidy correctness
- fee accounting
- UTXO existence and spendability
- duplicate input and duplicate spend rejection
- canonical ownership binding
- Falcon-512 witness verification

### 7.1. Validation Pipeline

```text
+------------------+    +---------------------+    +----------------------+
| Raw block bytes  | -> | Header + size check | -> | Transaction decode   |
+------------------+    +---------------------+    +----------------------+
                                                         |
                                                         v
                              +------------------------------------------------------+
                              | Duplicate-spend checks, UTXO reads, ownership locks, |
                              | witness parsing, Falcon verification, fee totals      |
                              +------------------------------------------------------+
                                                         |
                                                         v
                           +-------------------------------------------------------------+
                           | Coinbase reward validation, Merkle commitments, chainwork, |
                           | and atomic chainstate commit                               |
                           +-------------------------------------------------------------+
```

### 7.2. Ownership Binding

Spendable UTXOs are tied to a canonical 32-byte lock digest. A valid Falcon signature alone is not enough to spend an output. The witness public key must also hash to the exact lock expected by the UTXO being spent. Legacy lock forms, malformed locking data, and noncanonical ownership structures are rejected.

### 7.3. Confirmation and Spendability Policy

In the current code:

- ordinary UTXOs use **6 standard confirmations** across networks
- coinbase outputs require **100 confirmations**

Those values are consensus-facing policy constants in the current repository and should be treated as authoritative until explicitly changed in code.

## 8. Transaction Security

ATHO is controlled through cryptographic keys and spent through signed transactions. A transaction consumes existing UTXOs and creates new ones. To authorize that spend, the owner produces a Falcon-512 witness over the canonical Atho signing message for the relevant input.

Deterministic transaction hashing matters because every node must compute the same transaction identity from the same bytes. Canonical serialization matters because consensus cannot depend on ambiguous representations. Malformed transactions must fail the same way on every node, whether they arrive from a peer, a wallet, a miner, or an API endpoint.

The current code reflects that design:

- strict raw-byte decoding
- rejection of trailing bytes in canonical paths
- canonical ownership binding
- exact witness parsing
- deterministic sighash generation
- fail-closed Falcon verification
- duplicate-spend prevention through UTXO accounting

## 9. Post-Quantum Security Comparison

| System Type | Common Examples | Signature Type | Quantum Risk Profile | Atho Difference |
|---|---|---|---|---|
| Traditional Bitcoin-style systems | Bitcoin, Litecoin, similar UTXO chains | ECDSA or Schnorr-family ECC | Strong in today's environment, but elliptic-curve assumptions face long-horizon theoretical quantum risk | Atho replaces elliptic-curve transaction signatures with Falcon-512 |
| Ethereum-style systems | Ethereum, EVM-based chains | ECDSA-family ECC | Similar long-horizon elliptic-curve exposure at the account-signature layer | Atho keeps a simpler payment base layer and uses Falcon-512 for spend authorization |
| Classical payment ledgers | Custodial ledgers, conventional digital ledgers | Varies by operator | Often depend on centralized trust and mixed cryptographic assumptions | Atho pushes validation to independently verifying full nodes |
| Post-quantum-aware systems | Emerging PQ-focused ledgers | Lattice- or hash-based signatures | Designed to reduce reliance on elliptic-curve assumptions | Atho belongs in this category through Falcon-512-based transaction authorization |
| Atho | Atho | Falcon-512 | Post-quantum-aware signature posture with SHA3-384 hash-based mining | Combines Proof-of-Work simplicity, canonical UTXO validation, and Falcon-512 witnesses |

Atho does not claim immunity to every future attack class. Its claim is narrower and stronger: the chain is designed to reduce dependence on known elliptic-curve signature assumptions in transaction authorization while preserving the operational clarity of a Proof-of-Work payment network.

## 10. Network Participants

### 10.1. Users

Users hold ATHO, receive payments, create transactions, and control the keys that authorize spends.

### 10.2. Miners

Miners secure ordering through SHA3-384 Proof-of-Work. They assemble candidate blocks, include valid transactions, and receive subsidy plus fees when their blocks pass full validation.

### 10.3. Full Nodes

Full nodes validate blocks and transactions independently, enforce emission and ownership rules, maintain chainstate, and reject invalid data even when it arrives with valid proof of work.

### 10.4. Wallets

Wallets derive keys, track UTXOs, construct transactions, estimate fees, sign witnesses, and protect seed or secret-key material.

### 10.5. Exchanges and Payment Providers

Integrators track deposits, withdrawals, and confirmation depth. Reliable integration requires running a full node or infrastructure that reproduces consensus behavior faithfully.

## 11. Use Cases

Atho is best understood as a payment and settlement chain with post-quantum-aware authorization. Natural use cases include:

- peer-to-peer payments
- merchant settlement
- exchange deposit and withdrawal infrastructure
- wallet-based value transfer
- miner-secured settlement
- high-precision fee markets
- post-quantum-aware payment rails for long-term planning

The design deliberately avoids broad claims about every possible application category. Its strongest fit is durable, auditable digital money movement.

## 12. Risk Disclosures

No serious blockchain system is free of risk. Atho still requires ongoing review in several areas:

- **software risk:** implementation bugs, state-transition bugs, or upgrade errors can still occur
- **cryptographic implementation risk:** Falcon-512 must be implemented, serialized, and verified correctly at all times
- **network bootstrapping risk:** new public networks need healthy peer discovery and resilient node distribution
- **mining centralization risk:** Proof-of-Work systems can concentrate if hashpower becomes too narrow
- **wallet/key-loss risk:** user-controlled keys remain a direct custody responsibility
- **exchange integration risk:** third-party integration quality varies and can create deposit or withdrawal risk
- **regulatory uncertainty:** digital asset infrastructure remains subject to changing legal treatment
- **long-term quantum-security research uncertainty:** post-quantum design choices still benefit from continuous external review
- **documentation drift risk:** public statements must stay aligned with the code or they can mislead operators and integrators

## 13. Roadmap

### Phase 1 - Core Protocol Hardening

- consensus validation tightening
- wallet signing correctness
- mempool policy hardening
- emission and value-flow enforcement
- testnet stability

### Phase 2 - Public Testnet Expansion

- broader node distribution
- explorer feedback
- wallet interoperability testing
- miner participation growth
- structured bug reporting

### Phase 3 - Mobile and Desktop Wallet Maturity

- send/receive refinement
- better address management
- deterministic backup and recovery flows
- fee estimation refinement
- confirmation UX improvements

### Phase 4 - Mainnet Readiness

- final consensus freeze
- genesis and bootstrap review
- reproducible release builds
- public documentation alignment
- independent security review
- launch coordination

### Phase 5 - Ecosystem Growth

- exchange integrations
- merchant tooling
- payment infrastructure
- documentation expansion
- contributor workflow maturity

## 14. Conclusion

Atho combines Proof-of-Work ordering, post-quantum-aware transaction authorization, strict UTXO validation, and high divisibility into a payment-focused Layer 1. Its strongest differentiator is not feature sprawl. It is the effort to keep the rules auditable, the validation deterministic, the cryptography modernized, and the base layer operationally understandable.

The current codebase does not yet reflect every historical planning claim that has circulated around the project. That is exactly why this white paper is written from the implementation upward. For serious infrastructure, code truth matters more than aspirational copy.

Atho is designed to become the platinum standard for secure, scarce, post-quantum digital payments.

## Appendix A. White Paper Change Log

- Rewrote the paper around the current repository implementation instead of older planning values.
- Removed version and date from the title treatment and kept author plus contact only.
- Replaced colorful flowchart expectations with black-and-white ASCII diagrams.
- Added an explicit code-grounded policy note at the front of the document.
- Added a current-code monetary policy section and 150-year emission schedule.
- Updated the paper to the current 100-second / 5 ATHO / 6-confirmation / 100-maturity policy.
- Added founder-hash header metadata to the technical discussion.
- Added consensus, validation, transaction security, risk, and roadmap sections aligned with the live repository.

## Appendix B. Assumptions Used in the 150-Year Supply Projection

1. The projection uses the code constants in `crates/atho-core/src/constants.rs`, `crates/atho-core/src/consensus/params.rs`, and `crates/atho-core/src/consensus/subsidy.rs`.
2. The target block cadence is the active code value of **100 seconds**, which yields **315,360 blocks per year**.
3. The projection measures cumulative subsidy issuance at the end of each listed year.
4. Annual emission figures reflect **subsidy only** and exclude transaction fees.
5. The schedule assumes no future consensus change to block cadence, reward logic, or tail behavior.
6. The current code has **no finite max-supply cap**, so percentage-of-cap and remaining-to-cap values are intentionally marked `N/A`.
7. Rounded ATHO values are displayed for readability, but the underlying consensus accounting is performed in integer atoms.

## Appendix C. Consistency Checklist

- [x] White paper updated
- [x] Abstract updated
- [x] Current cryptography comparison added
- [x] Falcon-512 explained
- [x] SHA3-384 explained
- [x] Current code values used throughout
- [x] 12 decimals used throughout
- [x] Monetary policy section added
- [x] 150-year supply schedule added to white paper
- [x] APA-style emission table included
- [x] Separate monetary policy attachment prepared
- [x] All assumptions listed
- [x] Code constants checked against written policy
- [x] Mismatches reported instead of silently normalized
- [x] Final document reviewed for terminology consistency

## Appendix D. Code and Policy Mismatches Requiring Founder Review

The following mismatches should be resolved before public release of the white paper as the single external policy source:

1. **Max-supply mismatch.** Current code still does not enforce a fixed cap. Any public statement claiming a hard 78,000,000 ATHO ceiling would be inaccurate unless the code changes.
2. **Public-website mismatch.** The official `atho.io` site currently describes a different monetary model than the repository currently enforces.
3. **Release-readiness mismatch.** Public release of this paper should follow an explicit founder decision on whether the perpetual-tail, no-cap code path is the intended canonical policy.

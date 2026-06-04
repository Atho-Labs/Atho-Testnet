# Atho Testnet

This branch is the public **Atho testnet-only** release track built from the latest Alpha codebase and hardening work. It keeps the repo focused on the public test network: one launcher, one README, the white paper, the Rust code, and the validation/runtime pieces needed to run nodes, wallets, mining, explorer reads, and transaction broadcast on testnet.

Mainnet-facing launch helpers, regnet launch helpers, and the large Alpha documentation/report surface are intentionally removed from this branch. Use `Atho-Alpha` for broader mainnet and internal engineering work.

- Website: <https://atho.io>
- Explorer: <https://atho.io/explore/>
- Testnet repo target: `Atho-Labs/Atho-Testnet`
- Release line: `v0.3.0`
- Public testnet peers:
  - `162.222.206.163:9100`
  - `74.208.219.116:9100`
- Public testnet API nodes:
  - `https://testnet-node1.atho.io/api/v1`
  - `https://testnet-node2.atho.io/api/v1`

## What This Branch Is

This is a **testnet product branch**, not a full multi-network source distribution. The consensus/runtime code remains the current Atho implementation, but the public repo surface is simplified around testnet operation and testnet support.

That means:

- `runtestnet.py` is the only public desktop launcher
- the README is testnet-specific
- the white paper is kept at the repo root
- broad Alpha audit/report files are removed from the public branch surface
- public testnet wallet/API flows are documented here instead of spread across many docs

## Requirements

- Rust and Cargo
- Python 3
- A normal C/C++ toolchain
- OpenCL headers are optional; if GPU prerequisites are missing, the launcher can still fall back to CPU-oriented builds

## Quick Start

Install Rust if `cargo` is not already available:

```bash
curl https://sh.rustup.rs -sSf | sh
```

Build:

```bash
cargo build
```

Run the public testnet client:

```bash
python3 runtestnet.py
```

That launcher starts the desktop client together with a managed local testnet node. If local binaries are missing or stale, the launcher can rebuild before handing off to `atho-qt`.

Useful launcher flags:

```bash
python3 runtestnet.py --dry-run
python3 runtestnet.py --rebuild
python3 runtestnet.py --data-dir ~/.atho-testnet
python3 runtestnet.py --network-overrides-local
```

## Node Commands

Run the node directly:

```bash
cargo run -p atho-node --bin athod -- --network testnet
```

Check node status:

```bash
cargo run -p atho-node --bin athod -- status --network testnet
```

Verify local runtime and genesis wiring:

```bash
cargo run -p atho-node --bin athod -- verify --network testnet
```

## Public API and Wallet Endpoints

The public testnet nodes expose read APIs plus transaction broadcast routes for desktop and mobile wallet flows.

Public API bases:

- `https://testnet-node1.atho.io/api/v1`
- `https://testnet-node2.atho.io/api/v1`

Common read routes:

- `GET /health`
- `GET /network`
- `GET /network/stats`
- `GET /blocks/<height-or-hash>`
- `GET /transactions/<txid>`
- `GET /mempool`

Public transaction submission routes now available for wallets:

- `POST /tx/broadcast`
- `POST /tx/sendraw`
- `POST /sendrawtransaction`

This matters for:

- desktop wallet send flows
- mobile wallet send flows
- explorer-backed wallet integrations
- testnet external app integration work

## White Paper

- [ATHO_WHITE_PAPER.pdf](ATHO_WHITE_PAPER.pdf)

## Validation Commands

Recommended quick validation pass:

```bash
python3 -m unittest tests.test_runtime_launcher
cargo check --workspace
cargo check --manifest-path fuzz/Cargo.toml --all-targets
```

## v0.3.0 Patch Notes

This release refreshes the public testnet-only distribution from the current Alpha codebase after the accounting, emission, wallet-confirmation, and stability updates. It is intended as the clean testnet reset line for the updated Atho rules.

### Accounting and Emission Update

- Switched public monetary accounting to Bitcoin-style E-8 units: `1 ATHO = 100,000,000 atoms`.
- Set the base fee policy to `1 atom/vbyte`, keeping normal payments cheap while preserving exact integer accounting.
- Set the standard dust threshold to `100 atoms`.
- Updated the block schedule to 100-second target blocks, 864 blocks/day, and a 1,260,000-block halving interval.
- Updated subsidy rules to start at `50.00000000 ATHO` per block, halve through Era 7, and then use a permanent `0.39062500 ATHO` tail reward from block `8,820,000`.
- Regenerated the testnet genesis/accounting path for the new model. Existing pre-v0.3.0 testnet databases should be treated as incompatible with this reset line.

### Confirmation and Wallet Policy

- Normal transactions are considered confirmed once included in a valid best-chain block.
- Coinbase rewards keep the hard consensus maturity rule of 100 confirmations before they can be spent.
- Wallet confirmation depth is now wallet/application policy instead of a hard consensus delay for normal transactions.
- The official wallet defaults to 3 confirmations and exposes a user setting for the desired normal-transaction confirmation depth.
- RPC/API balance and UTXO flows expose confirmation filtering so external wallets, merchants, and apps can choose their own risk level.

### Stability and Address Safety

- Hardened Qt layout sizing so non-finite widget geometry cannot trigger the egui hit-test panic seen in the desktop client.
- Added network-mismatch protection before attaching a wallet to the Qt session.
- Made configured mining reward addresses network-scoped, so a valid address from another network is ignored/reset before startup mining paths can use it.
- Improved Qt test isolation so wallet tests do not read the operator’s real wallet storage.

### Explorer, API, and Testnet Operations

- Updated node explorer/API monetary formatting to report E-8 atom values consistently.
- Verified public testnet supply and genesis coinbase accounting as `50.00000000 ATHO` / `5,000,000,000 atoms`.
- Kept the public testnet nodes as full-node services, with mining controlled by local/operator clients.
- Refreshed the bundled white paper at the repo root with the new accounting, emission, block size, TPS, and confirmation-policy information.

### Release Surface

- Kept this repo testnet-only: `runtestnet.py`, one README, one white paper, Rust code, validation tests, runtime scripts, and fuzz targets.
- Kept `runmainnet.py`, `runregnet.py`, and the large Alpha engineering/report documentation out of this public testnet release.
- Published as `v0.3.0` because the accounting/emission model changes require a clean testnet generation boundary.

## v0.2.0 Patch Notes

This stable testnet release folds the latest Alpha hardening work into a simplified public testnet distribution. The goal is to carry forward the consensus/runtime/security improvements without making operators, wallets, or testers sort through mainnet-specific surface area.

### Consensus and Security Hardening

- Removed fee metadata from consensus trust assumptions by recomputing fees from committed transactions during validation instead of trusting uncommitted block-side totals.
- Bound transaction PoW fields into the committed transaction identity path so alternate raw transactions cannot share the same effective block commitments while differing in consensus-relevant tx-PoW bytes.
- Tightened coinbase rules so witness and tx-PoW edge cases are no longer left loose in consensus-critical validation.
- Added a future timestamp ceiling so miners cannot push block timestamps arbitrarily far ahead.
- Hardened binary decoding paths to reject non-canonical shortened transaction encodings that previously defaulted tx-PoW fields.
- Added parser allocation sanity rails to reduce malformed-count memory abuse risk in transaction and block decoding.
- Removed publicly derivable mining reward key behavior from real mainnet/testnet payout handling; public mining now uses explicit configured payout addresses instead of predictable reward secrets.

### Sync, Reorg, and Storage Work

- Improved branch reconstruction and cross-peer side-branch recovery so higher-work forks can be rebuilt even when blocks arrive out of order or from different peers.
- Hardened reorg recovery against locally mined isolated forks and archived side-branch replay cases.
- Expanded crash/fault coverage around snapshot commit, chainstate mutation, and replace-path recovery so failed state transitions are less likely to leave ambiguous persistence outcomes.
- Tightened startup recovery checks by replaying canonical history against persisted UTXO state instead of trusting only tip/snapshot metadata.
- Locked down `dev_seed_chainstate` and other sharp dev-only helpers so they are no longer normal production-facing surfaces.

### API and Wallet Integration Updates

- Enabled the public testnet wallet broadcast endpoints needed for external wallets and mobile wallet transaction submission.
- Kept the public testnet API nodes on HTTPS and validated they expose the correct `atho-testnet` identity.
- Preserved read-only public explorer/API flows while allowing controlled public testnet transaction relay.

### Explorer and Website Fixes

- Fixed the explorer’s stale expected testnet genesis hash so the public API no longer gets falsely flagged as the wrong network.
- Fixed the explorer and homepage uptime display so uptime is derived from canonical uptime seconds and continues advancing even when block production is quiet.
- Rebuilt the public website upload bundle and synced those explorer fixes into the website repository.

### UI and Runtime Fixes

- Patched the Qt settings/miner layout crash caused by non-finite egui sizing values reaching hit-test code.
- Preserved the managed local-node launch flow while simplifying the public launcher surface down to `runtestnet.py`.

### Release Surface Simplification

- Removed `runmainnet.py` and `runregnet.py` from this branch.
- Removed the large Alpha doc/report surface from this branch, keeping the public repo focused on testnet use.
- Rewrote the README for testnet operators, testers, explorers, and wallet integrators.

## Notes

- This branch is intentionally testnet-focused.
- Quiet block gaps should not be interpreted as explorer or chain uptime loss.
- For broad multi-network development and the full Alpha engineering surface, use the `Atho-Alpha` repository instead.

## License

Atho is licensed under the Apache License 2.0. See [LICENSE](LICENSE).

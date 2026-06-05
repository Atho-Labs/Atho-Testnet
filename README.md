# Atho Testnet

This branch is the public **Atho testnet-only** release track built from the latest Alpha codebase and hardening work. It keeps the repo focused on the public test network: one launcher, one README, the white paper, the Rust code, and the validation/runtime pieces needed to run nodes, wallets, mining, explorer reads, and transaction broadcast on testnet.

Mainnet-facing launch helpers, regnet launch helpers, and the large Alpha documentation/report surface are intentionally removed from this branch. Use `Atho-Alpha` for broader mainnet and internal engineering work.

- Website: <https://atho.io>
- Explorer: <https://atho.io/explore/>
- Testnet repo target: `Atho-Labs/Atho-Testnet`
- Release line: `v0.3.3`
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

## v0.3.3 Patch Notes

This patch is a follow-up hardening pass for the `v0.3.2` sync-loop fix. It keeps corrected sync targets fast by dropping stale queued download work above the newly proven live tip.

### Stale Work Pruning

- Added downloader support for forgetting pending, in-flight, completed, and hinted block work in one cleanup path.
- When terminal headers lower a stale sync target, the node now prunes header queues, compact-block work, validation state, and body-download work above the corrected live height.
- This prevents old unreachable target work from wasting requests or keeping the sync engine looking busy after the node has already learned the peer's real terminal height.

### Regression Coverage

- Added downloader coverage for removing pending, in-flight, completed, and hinted hashes.
- Extended terminal-header sync coverage to prove stale above-target in-flight work is cleared when the live target is corrected.

## v0.3.2 Patch Notes

This patch fixes the sync loop seen after `v0.3.1`, where a node could download to a live peer's terminal height and then keep re-requesting headers forever instead of accepting that the stale advertised target was unreachable.

### Sync Loop Fix

- Cleared stale peer chainwork whenever a peer's remembered tip changes after the version handshake. This prevents old version-handshake work from poisoning later terminal-header sync decisions.
- Stopped treating a different same-height tip hash as a required sync target unless the peer proves higher cumulative work.
- Kept same-height higher-work peers as a real sync target, so heavier forks still force catch-up.
- Lowered stale sync targets immediately when terminal headers or idle empty-header responses prove the live peer set is shorter than an old advertised target.

### Regression Coverage

- Added coverage for stale terminal headers lowering the target without waiting for the watchdog.
- Added coverage for idle empty-header responses not looping forever.
- Added coverage for equal-height/equal-work fork tips staying usable while same-height/higher-work peers still force sync.

## v0.3.1 Patch Notes

This patch release carries the Alpha fork-healing and sync-readiness fixes into the public testnet-only distribution. It is meant for node operators and wallet testers who saw nodes appear synced while they were actually sitting on a same-height fork or unresolved side branch.

### Fork Recovery and Chain Selection

- Made P2P sync targets fork-aware by tracking the best advertised height, tip hash, and chainwork together.
- Treated same-height competing tips as unresolved until headers and blocks prove the local tip satisfies the best known work target.
- Preserved peer tip hash and chainwork in live peer snapshots so reconnect and disconnect recovery does not collapse back to height-only sync decisions.
- Kept side-branch/orphan recovery active while a fork is unresolved, allowing the node to keep requesting headers and missing parents instead of reporting a false synced state.

### Mining, API, and Wallet Readiness

- Updated node sync readiness so `safe_to_mine` and `safe_to_serve` stay false while the local tip is behind the best known fork target, even when the heights match.
- Updated RPC, HTTP API, daemon status, and Qt wallet status to use fork-aware `safe_to_serve` rather than height-only checks.
- Prevented the desktop client from showing synced or starting wallet-ready flows on an unresolved same-height fork.

### Regression Coverage

- Added regression coverage for same-height preferred fork tips not being considered synced or safe to mine.
- Added P2P regression coverage for same-height peers with higher chainwork forcing header sync, while weaker same-height peers do not disturb the local tip.

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

# Atho Testnet

This branch is the public **Atho testnet-only** release track built from the latest Alpha codebase and hardening work. It keeps the repo focused on the public test network: one launcher, one README, the white paper, the Rust code, and the validation/runtime pieces needed to run nodes, wallets, mining, explorer reads, and transaction broadcast on testnet.

Mainnet-facing launch helpers, regnet launch helpers, and the large Alpha documentation/report surface are intentionally removed from this branch. Use `Atho-Alpha` for broader mainnet and internal engineering work.

- Website: <https://atho.io>
- Explorer: <https://atho.io/explore/>
- Testnet repo target: `Atho-Labs/Atho-Testnet`
- Release line: `v0.3.6`
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

## v0.3.6 Patch Notes

This patch hardens public testnet mining against stale block templates during forks and fast network-tip movement.

### Stale Template Mining Safety

- Blocks `getblocktemplate` while the local node is still syncing, has no ready public peer, is behind the advertised network target, or is not safe to mine.
- Rejects stale `submitblock` requests before they can enter chainstate when the submitted block no longer extends the active local tip.
- Returns structured `ATHO-BLK-007` stale-template details with submitted height, expected height, submitted parent, current tip, local height, sync target, header sync, and safe-to-mine state.
- Keeps `getmininginfo` explicit with `chain_synced`, `mining_allowed`, and `mining_blocked_reason` so operators can see exactly why mining is paused.

### Wallet Miner Fork Recovery

- Stops the desktop miner when the advertised sync target moves ahead of the local chain, even if the local tip hash has not changed yet.
- Restarts mining from a fresh template instead of finishing work on an obsolete parent.
- Adds stale-template logging with local height, sync target, header sync, safe-to-mine, current tip, and solved block hash when available.

### Regression Coverage

- Added a regression proving stale RPC-mined blocks are rejected before they can create a local RPC-side fork.
- Added a regression proving mining templates are paused when a public peer target is above the local tip.
- Added a Qt regression proving advertised target movement invalidates active mining work.

## v0.3.5 Patch Notes

This patch hardens testnet sync against damaged or stale bootstrap nodes that advertise a high height but cannot serve headers through that height.

### Advertised Height Safety

- Tracks the height a peer advertised during handshake separately from the terminal header height it actually served.
- Marks peers inconsistent when they advertise a higher height than they can serve.
- Refuses to lower the global sync target from a peer that proved `terminal_headers_below_advertised`.
- Stops re-requesting headers from that inconsistent peer while preserving the higher unresolved target for healthy peers.
- Keeps `chain_synced=false` and `safe_to_serve=false` while a connected peer has an unresolved advertised height above the local tip.

### Operator Diagnostics

- Adds RPC/status fields for `best_advertised_peer_height`, `best_serviceable_peer_height`, `unresolved_advertised_height`, `inconsistent_peer_count`, `healthy_sync_peer_count`, and `sync_warning`.
- Adds a topology warning when peers advertise heights they did not serve.
- Sets `chain_validation_status=peer_advertised_unserved_height` for this condition so clients and explorers do not report clean sync.

### Regression Coverage

- Added a regression proving a peer that advertises height `10` but serves terminal headers only through `2` does not lower the sync target, is penalized, does not receive another `getheaders`, and leaves the node unsafe to serve.
- Re-ran stale-target recovery tests to prove valid stale-target lowering still works when the higher target was not advertised by the terminal peer.

## v0.3.4 Patch Notes

This patch fixes the remaining terminal-header sync loop where a node could reach the advertised target height, receive `0` additional headers, and still keep re-requesting headers because stale same-height peer metadata survived maintenance.

### Terminal Header Sync Fix

- Treats an empty `Headers` response as a valid end-of-sync marker when the local database tip is already at or above the advertised target and there is no pending header, body, compact-block, or side-branch work.
- Re-primes the relay sync target from the local database tip and local chainwork on that terminal empty response.
- Updates the peer's remembered tip to the local tip when the peer proves there are no more headers, clearing stale same-height fork fingerprints that could restart the loop.
- Stops considering a different same-height tip hash as a reason to re-request headers unless the peer proves higher cumulative chainwork.

### Regression Coverage

- Added a regression for the `local_height == target_height`, `headers count == 0` case to prove maintenance does not queue another `getheaders` request afterward.
- Re-ran the stale-target regression suite to keep the previous `v0.3.2` and `v0.3.3` sync-loop fixes covered.

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

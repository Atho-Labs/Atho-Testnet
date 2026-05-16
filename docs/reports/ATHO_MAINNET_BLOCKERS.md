# Atho Mainnet Blockers

This is the short, direct list of issues that still block a mainnet-quality release.

## 1. Wallet plaintext persistence is still possible

- Area: `crates/atho-wallet/src/wallet/datafile.rs`
- Severity: High
- Why it matters: an empty password can still select plaintext persistence for wallet secrets.
- Fix: require encryption for persisted private material or make explicit dev-only plaintext mode impossible in production builds.
- Verify: add tests proving empty-password saves are rejected or encrypted.

## 2. Mainnet peer discovery is not provisioned

- Area: `crates/atho-p2p/src/config.rs`
- Severity: High
- Why it matters: mainnet DNS seeds and bootstrap peers are empty.
- Fix: define and validate real mainnet seed infrastructure.
- Verify: launch clean nodes and confirm autonomous peer discovery/sync.

## 3. Genesis/history policy still needs an explicit pre-mainnet decision

- Area: `crates/atho-core/src/genesis.rs`
- Severity: High
- Why it matters: genesis reward scripts are still legacy-form 48-byte scripts while the active ownership model is strict canonical 32-byte lock binding.
- Fix: before any real launch, either regenerate the network history/genesis or explicitly document and test the intended burn/unspendable semantics.
- Verify: chain bootstrap, spendability assumptions, and supply accounting match the chosen policy.

## 4. Mempool resource bounds are still too thin

- Area: `crates/atho-node/src/mempool.rs`
- Severity: High
- Why it matters: the mempool does not yet present a strong explicit memory cap / expiry / eviction contract.
- Fix: add bounded memory policy, expiry, eviction, and benchmark coverage.
- Verify: spam/load tests show bounded growth and stable admission behavior.

## 5. End-to-end benchmark tooling is not stable enough

- Area: `crates/atho-node/src/bin/atho-benchmark.rs`
- Severity: Medium
- Why it matters: the benchmark harness hung during this pass, which blocks clean release-performance signoff.
- Fix: debug and stabilize the harness or replace it with narrower repeatable benches.
- Verify: repeated benchmark runs finish cleanly and produce comparable results.

## 6. Public API deployment still depends on external controls

- Area: `crates/atho-node/src/api.rs`
- Severity: Medium
- Why it matters: the HTTP API has safe local defaults, but no built-in auth layer.
- Fix: keep the API loopback-only by default and require reverse-proxy/auth controls for any public deployment, or add first-party auth.
- Verify: deployment guide and operator config match the intended public-exposure model.

## 7. Workspace-wide strict clippy gating still fails in vendored Falcon crates

- Area: `Falcon 512 rs/`
- Severity: Medium
- Why it matters: clean release engineering is harder when strict lint gates fail outside first-party code.
- Fix: isolate vendored code from strict lint gating, patch locally, or vendor a cleaner upstream snapshot.
- Verify: targeted or workspace lint gates pass with the chosen policy.

## Mainnet Verdict

**Do not launch mainnet yet.**

The consensus core is much healthier than it was, but the remaining blockers are real enough that a mainnet claim would be premature.

# Atho Extended Testnet Readiness

## Verdict

**Safe for extended testnet.**

That is the right level for the current codebase after this pass. Atho is strong enough for broader public testnet use, adversarial sync/relay observation, wallet/operator feedback, and throughput tuning. It is not yet ready for real-value mainnet deployment.

## Why Extended Testnet Is Reasonable Now

- Consensus validation is strict and significantly less permissive than earlier in the project.
- Canonical payment-lock ownership binding is enforced.
- Raw transaction submission rejects noncanonical bytes and legacy lock formats.
- P2P framing now rejects trailing bytes cleanly.
- Storage regression coverage is strong and the full `atho-storage` suite passed.
- Launcher flow, API health, and node status surfaces work.
- Peer health and sync topology scoring already exist, which makes public network observation more useful.

## Risks That Still Need Monitoring

### 1. Wallet secret persistence policy

Empty-password wallet persistence is still not where it needs to be for mainnet. For extended testnet, this is manageable if operators understand that wallets with real value should not rely on this behavior.

### 2. Mempool resource behavior under spam

The mempool still needs clearer hard caps, eviction, and expiry behavior. During extended testnet, watch for:

- memory growth
- admission latency
- template build slowdown
- API mempool query slowdown

### 3. Peer quality and sync throughput

Mainnet seeding is not ready, but extended testnet can still validate:

- peer backoff behavior
- stale-peer handling
- topology health
- sync progress under mixed peers

### 4. Benchmark harness reliability

The current end-to-end benchmark binary did not terminate cleanly in this pass. That does not block testnet usage, but it does reduce visibility into throughput regressions.

### 5. Genesis/history policy

Legacy-form genesis reward scripts still exist in chain history. Extended testnet is the right place to settle whether the network will be reset or whether those outputs are intentionally treated as unspendable historical artifacts.

## What To Monitor During Extended Testnet

- block propagation latency
- transaction relay latency
- sync-from-zero duration
- mempool growth under bursty load
- orphan/stale-peer behavior
- explorer endpoint latency
- wallet scan and restore time
- API rate-limit usefulness
- database growth and restart behavior

## Recommended Extended Testnet Checklist

- [x] Canonical ownership binding enforced
- [x] Legacy lock spends rejected
- [x] Noncanonical raw transaction bytes rejected
- [x] Storage regression suite passed
- [x] Launcher flow validated
- [x] API health/status surfaces validated
- [ ] Mempool memory policy hardened
- [ ] Mainnet seeding provisioned separately
- [ ] Wallet plaintext persistence removed
- [ ] End-to-end benchmark harness stabilized

## Final Note

Extended testnet should be used aggressively here: it is the right environment to measure relay performance, sync durability, mempool pressure, wallet UX, and explorer behavior before anyone should even say the word "mainnet" with a straight face.

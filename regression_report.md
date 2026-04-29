# Atho Regression Report

## Regression Test Matrix
| Test | Expected | Actual | Result |
|---|---|---|---|
| `cargo test -p atho-storage --lib` | Pass | Pass | Pass |
| `cargo test --workspace --release --all-features` | Pass | Pass | Pass |
| `cargo test -p atho-storage higher_fee_transactions_are_accepted_in_blocks -- --exact` | Pass | Pass | Pass |
| `cargo test -p atho-storage wrong_public_key_for_standard_output_is_rejected -- --exact` | Pass | Pass | Pass |
| `cargo run --release -p atho-node --bin atho-attack -- --network regnet` | `19/19` pass | `19/19` pass | Pass |
| `cargo run --release -p atho-node --bin atho-adversarial -- --cases 52000 --seed 12345` | No unexpected accepts/rejects, no panics, no mismatches | `campaign_unexpected_accept=0`, `campaign_unexpected_reject=0`, `campaign_panics=0`, `campaign_mismatches=0` | Pass |
| `cargo run --release -p atho-node --bin athod -- wipe --network regnet --data-dir <tmp> --all` | Succeeds on sandbox tree | Succeeded | Pass |
| `cargo run --release -p atho-node --bin athod -- wipe --network mainnet --data-dir /tmp/... --all` | Refuse dangerous mainnet wipe | Refused | Pass |

## Failed Regressions
- None remain.

## Historical Regression Findings During This Audit
- Wrong-key standard output validation initially failed because 32-byte payment digests were not bound to the witness public key.
- Higher-fee block acceptance initially failed because block validation reused the exact-fee helper with a minimum-fee value.
- Both issues are now fixed and covered by regression tests.

## Regression Artifacts
- `crates/atho-storage/src/validation.rs`
- `crates/atho-node/src/bin/atho-attack.rs`
- `crates/atho-node/src/bin/atho-adversarial.rs`


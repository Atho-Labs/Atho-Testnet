# Atho Fuzz Report

## Fuzz Targets
| Target | Runtime | Inputs Tested | Crashes | Unique Failures | Status |
|---|---:|---:|---:|---:|---|
| `tx_witness_parse` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |
| `p2p_frame_decode` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |
| `p2p_message_roundtrip` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |
| `rpc_request_decode` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |
| `tx_decode` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |
| `tx_roundtrip` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |
| `sighash` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |
| `block_decode` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |
| `block_validate` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |
| `mempool_admission` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |
| `compact_block_reconstruct` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |
| `network_message_decode` | Direct libFuzzer binary | 1,000 | 0 | 0 | Pass |

## Crash Artifacts
- Artifact: none
- Reproduction command: not applicable
- Status: no crashes observed

## Notes
- `cargo fuzz` is not installed in this environment, so the repo's fuzz binaries were run directly with `cargo run --manifest-path fuzz/Cargo.toml --bin <target> -- -runs=20000`.
- The libFuzzer binaries emitted the usual warning about missing sanitizer hooks in this environment, but all runs completed cleanly.
- The runs did not produce unique failures or interesting crash inputs.
- The newly added consensus fuzz targets were smoke-run directly with `cargo run --release --manifest-path fuzz/Cargo.toml --bin <target> -- -runs=1000`.
- The consensus smoke runs completed cleanly with 0 crashes and 0 unique failures.

## Fuzz Coverage Gaps
- No sanitizer-backed fuzz CI.
- No corpus persistence workflow for the fuzz targets yet.
- No fuzzing harness for long-running corpus minimization or crash reproduction in CI yet.

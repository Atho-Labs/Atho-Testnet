# Atho Fuzz Report

## Fuzz Targets
| Target | Runtime | Inputs Tested | Crashes | Unique Failures | Status |
|---|---:|---:|---:|---:|---|
| `tx_witness_parse` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |
| `p2p_frame_decode` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |
| `p2p_message_roundtrip` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |
| `rpc_request_decode` | Direct libFuzzer binary | 20,000 | 0 | 0 | Pass |

## Crash Artifacts
- Artifact: none
- Reproduction command: not applicable
- Status: no crashes observed

## Notes
- `cargo fuzz` is not installed in this environment, so the repo's fuzz binaries were run directly with `cargo run --manifest-path fuzz/Cargo.toml --bin <target> -- -runs=20000`.
- The libFuzzer binaries emitted the usual warning about missing sanitizer hooks in this environment, but all runs completed cleanly.
- The runs did not produce unique failures or interesting crash inputs.

## Fuzz Coverage Gaps
- No dedicated `tx_decode` target.
- No dedicated `tx_roundtrip` target.
- No dedicated `sighash` target.
- No dedicated `block_decode` target.
- No dedicated `block_validate` target.
- No dedicated `mempool_admission` target.
- No dedicated `compact_block_reconstruct` target.
- No dedicated `network_message_decode` target.
- No sanitizer-backed fuzz CI.
- No corpus persistence workflow for the missing consensus targets yet.

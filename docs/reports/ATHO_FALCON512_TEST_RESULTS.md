# Atho Falcon-512 Test Results

## Commands Run

Passed:

- `cargo test -p atho-crypto falcon -- --nocapture`
- `cargo test -p atho-crypto secret_bytes_debug_is_redacted -- --nocapture`
- `cargo test -p atho-wallet mnemonic_debug_is_redacted -- --nocapture`
- `cargo test -p atho-wallet wallet_seed_debug_is_redacted -- --nocapture`
- `cargo test -p atho-wallet wallet_debug_does_not_expose_mnemonic_or_seed_material -- --nocapture`
- `cargo test -p atho-wallet wallet_restore_reproduces_deterministic_addresses -- --nocapture`
- `cargo test -p atho-wallet wallet_passphrase_changes_root_material -- --nocapture`
- `cargo test -p atho-wallet wallet_datafile_permissions_are_owner_only -- --nocapture`
- `cargo test -p atho-storage cross_network_signature_replay_is_rejected_even_with_valid_local_pow -- --nocapture`
- `cargo test -p atho-storage wrong_public_key_for_standard_output_is_rejected -- --nocapture`
- `cargo fmt --check`
- `cargo check --workspace`

## Key Outcomes

- Falcon core tests: `12 passed`
- new malformed-input and replay-boundary regressions: passed
- wallet debug-redaction regressions: passed
- wallet deterministic behavior after zeroizing-buffer change: passed
- storage-level wrong-network and wrong-key spend regressions: passed

## Notes

- No dedicated Falcon fuzz harness was run because the repo does not currently include one.
- No consensus behavior changes were introduced in this pass.

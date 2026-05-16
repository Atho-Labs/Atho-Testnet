# Atho Falcon-512 Hardening Plan

## Priority 0: Production Blocker

### Bind every accepted spend script to the correct Falcon key

- **Issue:** [validation.rs](/home/ano/Desktop/Atho-Testnet-main/crates/atho-storage/src/validation.rs:447) only enforces Falcon public-key ownership for 32-byte digest scripts.
- **Needed change:** explicitly define the supported script forms and reject or properly validate everything else.
- **Risk:** consensus-impacting. Needs careful rollout.
- **Tests to add:**
  - wrong-key spend of nonstandard script fails
  - supported legacy script, if intentionally retained, verifies exactly one canonical ownership rule

## Priority 1: Wallet Secret Protection

### Remove plaintext wallet persistence as the easy path

- Require an explicit flag for plaintext wallet save mode, or reject empty passwords by default.
- Keep AES-256-GCM path as the normal operator flow.

### Expand zeroization review

- Review:
  - decrypted wallet payload buffers
  - mnemonic passphrase handling
  - any long-lived seed-derived intermediate buffers

## Priority 2: Falcon Parser and Fuzz Coverage

### Add fuzz targets

- Falcon public key constructor
- Falcon secret key constructor
- witness parser
- transaction verification with malformed grouped witness data

### Add malformed throughput benchmarks

- valid verify
- malformed key reject
- malformed signature reject
- grouped-input signing digest verify

## Priority 3: Performance Without Security Loss

### Avoid full secret-key derivation for address-only paths

- Current address derivation builds a full deterministic Falcon keypair.
- Safer long-term improvement: derive or cache the public side without constructing a full secret key when the wallet only needs an address preview.

### Consider a bounded exact-key verification cache only if profiling justifies it

- Cache key must include:
  - network context
  - public key bytes
  - signing digest
  - signature bytes

## Priority 4: Side-Channel and Deployment Review

- Review upstream `fn-dsa` side-channel assumptions and document them in Atho docs.
- Keep signing isolated to wallet-local code paths.
- Reconfirm no node API endpoint ever grows a signing shortcut.

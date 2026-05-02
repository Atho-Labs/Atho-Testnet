# Atho Code Commenting Standard

## Purpose

This guide defines how comments and documentation should be added across the Atho codebase without making the code noisy or misleading.

The goal is to explain:

- why a module exists
- what invariant a type or function protects
- whether code is consensus-critical, policy-only, storage-dangerous, or wallet-secret
- what assumptions future maintainers must preserve

## Comment Types

Use module-level docs for important files:

```rust
//! Module summary.
//!
//! Explains why this file exists and where it sits in the trust boundary.
```

Use Rust doc comments for public APIs:

```rust
/// Public API summary.
///
/// # Security
/// Explain what abuse or misuse is prevented.
///
/// # Consensus
/// Explain whether the result must remain deterministic across all nodes.
```

Use normal comments for non-obvious internals:

```rust
// SECURITY: Reject duplicate inputs before fee accounting.
// PERFORMANCE: Avoid extra allocations on the hot validation path.
// STORAGE: These writes must commit atomically with the tip snapshot.
```

## Labels

Use labels when they materially help review:

- `CONSENSUS`
- `SECURITY`
- `STORAGE`
- `WALLET SECURITY`
- `PERFORMANCE`
- `POLICY`
- `INVARIANT`
- `WARNING`
- `TODO`

## What Good Comments Do

Good comments explain the rule behind the code.

Bad:

```rust
// Check signature.
```

Good:

```rust
// CONSENSUS: Verify the Falcon witness against the canonical signing digest.
// If signing and verification use different byte layouts, every spend fails.
```

## What To Avoid

Do not add comments that:

- repeat obvious syntax
- speculate about behavior you have not verified
- describe policy checks as consensus checks
- expose secrets, seeds, mnemonics, or private keys
- go stale because they mention old constants or old network names

## Hot Path Rule

On block validation, transaction validation, hashing, signature verification, UTXO lookup, mempool admission, and relay paths:

- prefer a few precise comments over many line-by-line comments
- explain performance assumptions once near the relevant block
- avoid comments that obscure the logic being optimized

## Review Checklist

Before adding or changing comments:

1. Verify the code path actually does what the comment claims.
2. Mark consensus-sensitive behavior clearly.
3. Distinguish relay policy from consensus.
4. Distinguish diagnostics helpers from production acceptance paths.
5. Remove stale comments instead of layering new comments on top of old ones.

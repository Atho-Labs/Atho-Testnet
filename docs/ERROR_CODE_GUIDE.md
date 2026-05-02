# Atho Error Code Guide

## Purpose

Atho uses structured `ATHO-*` error codes so operators and developers can identify failures quickly without relying on vague text.

## Code Shape

Typical shape:

- `ATHO-CFG-*`
- `ATHO-NET-*`
- `ATHO-CONS-*`
- `ATHO-BLK-*`
- `ATHO-TX-*`
- `ATHO-MEM-*`
- `ATHO-DB-*`
- `ATHO-RPC-*`
- `ATHO-WALLET-*`
- `ATHO-MINE-*`

## Usage Rules

When returning an error:

1. choose the specific registry descriptor
2. keep the public message safe
3. attach safe technical detail only when it helps diagnosis
4. do not leak secrets or private wallet material

## Commenting Rule

When an error path is important, explain:

- what caused the error
- whether it is consensus-critical
- whether it is policy-only
- what system boundary produced it

Example:

```rust
// Error ATHO-TX-014 is raised when Falcon witness verification fails against
// the canonical transaction signing digest.
```

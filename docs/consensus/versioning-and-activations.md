# Versioning and Activations

## Purpose

Atho includes explicit version and activation scaffolding now so future rule changes do not become ad hoc runtime behavior later.

## Current Version Constants

Implemented in:

- `crates/atho-core/src/consensus/rules.rs`

Current values:

| Item | Value |
| --- | ---: |
| Protocol version | `1` |
| Storage schema version | `3` |
| Ruleset version V1 | `1` |
| Block version V1 | `1` |
| Transaction version V1 | `1` |
| V1 activation height | `0` |

Inactive placeholders:

| Item | Value |
| --- | ---: |
| Ruleset version V2 placeholder | `2` |
| Block version V2 placeholder | `2` |
| Transaction version V2 placeholder | `2` |
| V2 activation height | `None` |

## Activation Model

The scheduling structure is:

- `ScheduledActivation`
- `rules_at_height(height)`
- `block_version_at_height(height)`
- `transaction_version_at_height(height)`

Why:

- one centralized height router prevents version logic from scattering across validation code

## Current Behavior

Today:

- V1 is active from genesis
- V2 is present as an explicit placeholder only
- block and transaction validation reject unsupported future versions

Implemented in:

- `crates/atho-storage/src/validation.rs`

## Storage Versioning

The storage layer also enforces:

- `STORAGE_SCHEMA_VERSION = 3`

On mismatch:

- persisted local state is rejected
- recoverable local state can be quarantined and rebuilt

Why:

- storage upgrades are operationally different from consensus upgrades and need their own explicit guardrail

## Design Rationale

Chosen:

- explicit constants
- explicit scheduled activation registry
- explicit support checks in validation

Avoided:

- magic numbers in validators
- silent post-height behavior changes
- UI-owned or runtime-owned version interpretation

## Current Limitations

- there is no active non-V1 ruleset yet
- there is no schema migration framework yet, only mismatch detection and recovery/quarantine
- there is no live activation boundary scenario in the public network layer

## Related Documentation

- [Consensus Rules](consensus-rules.md)
- [Current Production Status](../production-readiness/current-status.md)

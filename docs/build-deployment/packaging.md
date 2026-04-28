# Build and Packaging

## Purpose

The packaging flow stages a minimal local release bundle without pretending to be a full distribution pipeline.

## Current Artifacts

The local package script stages:

- `athod`
- `atho-mine`
- `atho-qt`
- `README.md`
- `COMMANDS.md`
- `RELEASE_NOTES.md`
- `PACKAGING.md`

Source locations:

- binaries from `target/release/`
- release documentation from `docs/production-readiness/release-notes.md`
- packaging notes from `docs/build-deployment/packaging.md`

## Local Release Build

```bash
cargo build --release -p atho-node -p atho-qt
```

Or use the staging helper:

```bash
./scripts/package.sh
```

## Why The Packaging Flow Is Small

Chosen:

- separate node and desktop binaries
- explicit staged output under `dist/release/`
- one canonical source for release docs

Avoided:

- generated duplicate source docs
- overbuilt packaging metadata before the runtime is fully production-ready

## Current Limitations

- no release signing pipeline
- no installer generation
- no reproducible-build attestation workflow
- no cross-platform distribution metadata automation

## Related Documentation

- [Release Notes](../production-readiness/release-notes.md)
- [Commands](../operations/commands.md)

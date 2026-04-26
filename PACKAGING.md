# Atho Packaging

## Build artifacts

- `athod` from `crates/atho-node`
- `atho-qt` from `crates/atho-qt`

## Local release build

```bash
cargo build --release -p atho-node -p atho-qt
```

## Staging layout

The package script stages binaries and release docs under `dist/`.

## Notes

- Keep the runtime binary and the desktop client separate
- Keep the package surface small and boring
- Use the release notes file as the baseline ship checklist


# GPU Mining

This guide covers the native FFI GPU path for Atho.

The design is:

- `atho-node` builds block templates and validates solved blocks
- `atho-gpu-native` owns the unsafe C++/OpenCL bridge
- `atho-mine` and `atho-qt` choose `cpu`, `gpu`, or `auto` at runtime
- `--backend gpu` requires a real OpenCL GPU
- `--backend auto` prefers GPU and falls back to CPU if GPU init or execution fails
- Atho reports the effective backend plus the detected device name/vendor/driver
- GPU probe and runtime failures now carry stable codes such as `ATHO-MINE-102` and `ATHO-MINE-103`
- the GPU backend only receives the canonical header bytes, nonce offset, target, and batch settings

## What You Need

Install a working C/C++ toolchain and an OpenCL runtime for your GPU.

Platform notes:

- macOS: Xcode Command Line Tools and the system OpenCL framework
- Linux: a C/C++ compiler, OpenCL headers, and your vendor OpenCL ICD/runtime
- Windows: Visual Studio Build Tools with C++ support and the vendor OpenCL SDK/runtime

If the OpenCL headers or runtime are missing, `gpu-native` builds will fail cleanly and CPU-only builds still work.
If GPU mining fails at runtime in `auto` mode, the app switches to CPU and surfaces the fallback reason in the miner status.

## Build

Build the standalone miner with GPU support:

```bash
cargo build -p atho-node --bin atho-mine --release --features gpu-native
```

Build the desktop client with GPU support:

```bash
cargo build -p atho-qt --release --features gpu-native
```

CPU-only builds do not need the feature flag:

```bash
cargo build -p atho-node --bin athod --release
```

## Run

Standalone miner, GPU backend:

```bash
./target/release/atho-mine --network regnet --backend gpu
```

Probe the detected GPU and driver details without mining:

```bash
./target/release/atho-mine --probe-gpu
```

That command also prints the structured unavailability code when probe fails, for example `gpu_unavailable_code=ATHO-MINE-102`.

Desktop client, GPU backend:

```bash
ATHO_MINING_BACKEND=gpu ./target/release/atho-qt --network regnet --local-node
```

Inside Qt, the same backend can be selected from `Settings > Mining`, and the panel shows the detected device, vendor, driver, and whether `auto` would fall back.
The same panel now also shows the probe code when GPU detection fails.

Windows PowerShell:

```powershell
$env:ATHO_MINING_BACKEND = "gpu"
.\target\release\atho-qt.exe --network regnet --local-node
```

If you want the miner to auto-select the accelerator when available:

```bash
./target/release/atho-mine --network regnet --backend auto
```

If you omit `--backend`, the miner defaults to `auto`, which prefers GPU and falls back to CPU.
Use `gpu` only when you want a hard failure instead of a silent CPU fallback.

## Regression Checks

Run the feature-enabled test suite before shipping a GPU build:

```bash
cargo test -p atho-node --features gpu-native
cargo test -p atho-qt --features gpu-native
```

## Environment

Useful tuning variables:

- `ATHO_MINING_BACKEND` selects `cpu`, `gpu`, or `auto`
- `ATHO_GPU_KERNEL_PATH` overrides the kernel source path
- `ATHO_GPU_BATCH_SIZE` sets the batch size used by the native helper
- `ATHO_GPU_MAX_BATCH` caps the accepted batch size

For production, keep the node authoritative and treat the GPU backend as a pure work executor.

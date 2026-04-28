#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist_dir="${root_dir}/dist"
target_dir="${dist_dir}/release"

mkdir -p "${target_dir}"

cargo build --release -p atho-node -p atho-qt --manifest-path "${root_dir}/Cargo.toml"

cp "${root_dir}/target/release/athod" "${target_dir}/athod"
cp "${root_dir}/target/release/atho-mine" "${target_dir}/atho-mine"
cp "${root_dir}/target/release/atho-qt" "${target_dir}/atho-qt"
cp "${root_dir}/README.md" "${target_dir}/README.md"
cp "${root_dir}/docs/operations/commands.md" "${target_dir}/COMMANDS.md"
cp "${root_dir}/docs/operations/launch-checklist.md" "${target_dir}/LAUNCH_CHECKLIST.md"
cp "${root_dir}/docs/production-readiness/release-notes.md" "${target_dir}/RELEASE_NOTES.md"
cp "${root_dir}/docs/build-deployment/packaging.md" "${target_dir}/PACKAGING.md"
cp "${root_dir}/docs/build-deployment/athod.service.example" "${target_dir}/athod.service.example"

echo "staged release artifacts in ${target_dir}"

#!/usr/bin/env bash
set -euo pipefail

roots=()
for root in \
  "$PWD" \
  "$HOME/Desktop/Atho-Alpha-main" \
  "$HOME/Desktop/Atho-Testnet-release" \
  "$HOME/Desktop/Atho-Testnet-main"; do
  if [[ -f "$root/Cargo.toml" ]]; then
    roots+=("$root")
  fi
done

unique_roots=()
for root in "${roots[@]}"; do
  seen=0
  for known in "${unique_roots[@]}"; do
    if [[ "$known" == "$root" ]]; then
      seen=1
      break
    fi
  done
  if [[ "$seen" -eq 0 ]]; then
    unique_roots+=("$root")
  fi
done

commands=(
  "cargo fmt --check"
  "cargo test -p atho-node headers_from_one_peer_keep_other_ready_peer_pipeline_full -- --nocapture"
  "cargo test -p atho-node headers_stage_only_near_tip_blocks_in_small_batches -- --nocapture"
  "cargo test -p atho-node low_peer_stale_block_request_retries_without_disconnect -- --nocapture"
  "cargo test -p atho-node block_refill_stays_on_current_peer_socket -- --nocapture"
  "cargo test -p atho-node buffered_out_of_order_block_frees_download_slot -- --nocapture"
  "cargo test -p atho-node stalled_headers_request_disconnects_peer_instead_of_spinning -- --nocapture"
  "cargo test -p atho-node far_ahead -- --nocapture"
  "cargo test -p atho-node compact_future_header_advances_sync_target_before_body_reconstruction -- --nocapture"
  "cargo test -p atho-p2p sync::tests -- --nocapture"
  "cargo test -p atho-p2p downloader::tests -- --nocapture"
  "cargo check -p atho-p2p -p atho-node"
)

for root in "${unique_roots[@]}"; do
  printf '\n== %s ==\n' "$root"
  for command in "${commands[@]}"; do
    printf '+ %s\n' "$command"
    (cd "$root" && eval "$command")
  done
done

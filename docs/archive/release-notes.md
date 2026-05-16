# Archived Document

This document is kept for historical reference only.
It may contain outdated information and should not be used as the current setup guide.

# Atho Testnet Release Notes

These notes were moved out of the main README during the repository cleanup pass.

## v0.1.8

- Added header-first block scheduling improvements from Alpha so peers can keep pipelined block request windows full without one-block-at-a-time stalls.
- Hardened header acceptance by rejecting wrong-network headers, out-of-bounds targets, and headers that fail their committed proof of work before full block download.
- Improved low-peer sync behavior so the only useful peer is retried for stale block responses instead of being disconnected for ordinary slowness.
- Added per-peer stale request requeue handling so timed-out blocks can be retried cleanly without stranding pending downloads.
- Added same-box sync regression coverage for Alpha, Testnet release, and the local Testnet checkout.

## v0.1.7

- Replaced peer-local fork buffering with a global side-branch pool indexed by block hash and parent hash, so fork chains can be reconstructed even when blocks arrive out of order from different peers.
- Hardened reorg recovery after bootstrap outages where local miners continued on an isolated fork and later need to switch back to a higher-work network branch.
- Preserved bounded side-branch storage while keeping low-height bridge blocks needed to reconnect a branch back to the canonical fork point.
- Dropped invalid assembled side branches after failed contextual validation so one bad fork candidate cannot poison later sync attempts.
- Added cross-peer side-branch regression coverage for rebuilding a higher-work fork over a local competing fork.

## v0.1.6

- Fixed fork recovery when a node already has winning-chain blocks archived locally but they are no longer canonical after mining on an isolated fork.
- Header serving now ignores archived side-branch locator hashes and only anchors responses to the node's canonical chain, preventing invalid header sequences after reorgs.
- Sync now replays known non-canonical blocks from local storage during header catch-up instead of skipping them as already downloaded.
- Branch buffering now preserves the low-height bridge back to the fork point and backfills known archived ancestors, preventing deep fork recovery from dropping the blocks needed to reconnect.

## v0.1.5

- Hardened fork recovery after bootstrap outages by building header sync locators from persisted chain history instead of only the recent in-memory reload window.
- Added periodic, relay-safe peer address sharing so connected nodes can organically learn `testnet-node2` and other healthy peers from the network.
- Seeded configured testnet bootstrap peers into the live discovery graph so bootstrap nodes can relay both public testnet peers to older connected clients.
- Tightened TCP sync regression tests around chain-sync readiness, real-socket reorg recovery, transaction relay, and peer address gossip.

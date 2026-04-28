# Launch Checklist

This is the final pre-launch checklist for Atho.

## Software Readiness

- [x] canonical consensus path is in place
- [x] transaction creation, signing, validation, and mining are exercised
- [x] block assembly, PoW validation, acceptance, and replay are exercised
- [x] mempool admission, conflict handling, and block removal are exercised
- [x] UTXO apply, rollback, maturity, and reload are exercised
- [x] wallet create/open/import/send/receive/history are exercised
- [x] Qt client follows the backend tip over the real RPC path
- [x] node restart and recovery paths are exercised
- [x] schema mismatch still fails closed
- [x] adversarial mutation campaign is green
- [x] standalone miner flow is green
- [x] headless node flow is green

## Operator Readiness

- [x] one primary command exists for the full node
- [x] one primary command exists for the miner
- [x] one primary command exists for the desktop client
- [x] default runtime roots are OS-native
- [x] explicit `--data-dir` override exists
- [x] RPC is local-only by default
- [x] P2P listens publicly by default
- [x] Windows quick-start exists
- [x] VPS full-node guide exists
- [x] release staging includes node, miner, client, and operator docs

## Deployment Readiness

- [x] node runs headless with explicit runtime paths
- [x] miner can pull templates from a node and submit a solved block
- [x] package staging works
- [x] systemd unit example exists
- [ ] VPS SSH host identity is verified out of band
- [ ] DNS seeds are added
- [ ] final public bootstrap peer list is chosen

## Security Gates Before Public Bring-Up

Do not proceed until all of these are true:

1. the VPS SSH host key is verified out of band
2. the deployment operator confirms the intended public P2P port exposure
3. the initial bootstrap peer plan is finalized
4. the public P2P wire path is hardened enough for internet exposure
5. DNS seeds are added only after the node software and deployment path are confirmed stable

## What Still Remains Before Public Launch

1. verify and update the VPS SSH host key for `74.208.219.116`
2. finish the remaining public-wire hardening needed for an internet-facing P2P bind
3. add DNS seeds
4. bring the network online

## Related Documentation

- [Runtime Model](runtime-model.md)
- [VPS Full Node](vps-full-node.md)
- [Commands](commands.md)
- [Current Production Status](../production-readiness/current-status.md)

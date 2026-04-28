# RPC and Client Backend

## Purpose

The RPC layer is the boundary between Atho’s backend state and its thin clients or automation tools.

Implemented in:

- `crates/atho-rpc/src/`
- `crates/atho-node/src/service.rs`
- `crates/atho-qt/src/connection.rs`

## Current RPC Requests

Current request types:

- `GetBlockCount`
- `GetNetwork`
- `GetNodeStatus`
- `GetBlockTemplate`
- `SubmitBlock`
- `SubmitTransaction`
- `ListUtxos`
- `GetMempoolInfo`
- `GetMempoolSpentInputs`

Why:

- the current surface is intentionally small and operationally focused

## Transport

Current transport:

- JSON line messages over TCP
- connection timeout: `5 seconds`
- read/write timeout: `10 seconds`
- maximum message size: `1 MiB`

Implemented in:

- `crates/atho-rpc/src/transport.rs`

Why:

- for a local-first client/backend boundary, simple framed JSON is easier to debug and stabilize than a custom binary API

## Runtime Security Defaults

Current operator defaults:

- RPC binds to loopback
- public RPC binds are refused unless the operator explicitly opts in
- desktop local-node startup stays on the same RPC path as external clients

Why:

- a private-by-default RPC surface is the correct operational default for VPS nodes and local desktop users

## Service Ownership

Mutable node actions are handled by `NodeService`, which:

- owns the orchestrator
- refreshes runtime views after accepted state changes
- provides node status snapshots
- bridges RPC request types to backend operations

Why:

- the RPC crate should not own the node; it should speak to a node-owned service surface

## Qt Interaction Model

The Qt client uses:

- status polling
- request/response submission for sends and block actions
- optional managed local-node startup

The Qt client supports:

- an in-process backend for tests
- a real RPC backend for runtime
- managed child-node startup for `--local-node`

Why:

- this keeps real runtime behavior on the RPC path while still allowing deterministic sandbox tests

## Current Limitations

- authentication is still local-development-oriented, not production-hardened
- there is no public API profile intended for hostile networks
- request-rate hardening is still light
- malformed-request fuzz coverage is started, but not yet broad

## Related Documentation

- [Qt Client](../gui-client/qt-client.md)
- [Commands](../operations/commands.md)
- [Troubleshooting](../operations/troubleshooting.md)

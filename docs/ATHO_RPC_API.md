# Atho RPC API

This document describes the current command-oriented RPC layer used by:

- `athod`
- `atho-cli`
- the Atho Qt debug console

## Transport

The current Atho RPC transport is a local JSON-line protocol over TCP.

- default bind: `127.0.0.1:<network rpc port>`
- request: one JSON object per line
- response: one JSON object per line
- local-only by default

The current transport is intended for local operator and client use. It is not yet a hardened public RPC interface.

## Command Request

Registry-backed commands are sent through:

```json
{
  "ExecuteCommand": {
    "name": "getblockchaininfo",
    "args": [],
    "confirmed": false
  }
}
```

Fields:

- `name`: command name
- `args`: positional string arguments
- `confirmed`: explicit confirmation for dangerous commands

## Command Success Response

Successful command execution returns:

```json
{
  "Command": {
    "command": "getblockchaininfo",
    "group": "blockchain",
    "permission": "PUBLIC_READ",
    "dangerous": false,
    "network": "atho-testnet",
    "data": {
      "network": "atho-testnet",
      "height": 12345
    }
  }
}
```

## Error Response

Failures return structured `RpcError` data:

```json
{
  "Error": {
    "code": "ATHO-RPC-002",
    "title": "Invalid RPC Request",
    "message": "The RPC request was malformed or missing required data.",
    "severity": "error",
    "details": "getblockhash expected height as unsigned integer"
  }
}
```

Consensus-related failures preserve their real Atho error code through the RPC boundary.

## Current Implemented Command Surface

The initial command router currently supports:

- `help`
- `getstatus`
- `gethealth`
- `getversion`
- `geterrorcodes`
- `getblockcount`
- `getbestblockhash`
- `getblockhash`
- `getblock`
- `getblockheader`
- `getblockchaininfo`
- `getnetworkinfo`
- `getconnectioncount`
- `getpeerinfo`
- `getmempoolinfo`
- `getblocktemplate`
- `gettemplateinfo`
- `getmininginfo`
- `getnetworkparams`
- `getgenesisinfo`
- `validateathoaddress`
- `sha3_384`

The source of truth for command metadata is:

- [`crates/atho-rpc/src/command.rs`](/Users/eyeanonymous/Desktop/Atho-Alpha /crates/atho-rpc/src/command.rs)

## Safety Model

- command routing never bypasses node validation
- submit paths still use the node's normal checked flows
- wrong-network or test-only restrictions are enforced at the service layer
- dangerous command confirmation is part of the invocation model even where the initial command set is read-focused

## Current Limitation

The transport layer does not yet implement:

- cookie auth
- username/password auth
- remote RPC hardening
- TLS

Until that lands, treat this RPC layer as a local operator/client interface only.

# Atho CLI Commands

`atho-cli` is the Bitcoin Core-style command client for Atho.

It talks to a running local Atho node over the existing JSON-line RPC transport and uses the same command registry as the Qt debug console.

## Current Scope

This is the initial production-facing slice:

- registry-backed command metadata
- structured JSON responses
- table / pretty / raw JSON output modes
- local command help without requiring a running node
- no consensus bypass

The Qt debug console now exposes the same registry with:

- grouped command browsing
- example insertion
- workflow shortcuts
- history navigation
- local `help` rendering even when the node is still offline

It does **not** yet expose the full long-term wallet/admin surface from the roadmap prompt.

## Usage

```bash
atho-cli [--network <mainnet|testnet|regnet|prunetest>] \
         [--rpc-url <host:port>] \
         [--format <json|pretty|table>] \
         [--confirm] \
         <command> [args]
```

Examples:

```bash
atho-cli help
atho-cli help getblocktemplate
atho-cli --network regnet getstatus
atho-cli --network testnet getblockchaininfo
atho-cli --format table getpeerinfo
atho-cli validateathoaddress A...
atho-cli sha3_384 ABC
```

## Implemented Commands

### Control

- `help [command|group]`
- `getstatus`
- `gethealth`
- `getversion`
- `geterrorcodes`

### Blockchain

- `getblockcount`
- `getbestblockhash`
- `getblockhash <height>`
- `getblock <hash|height>`
- `getblockheader <hash|height>`
- `getblockchaininfo`
- `getnetworkparams`
- `getgenesisinfo`

### Network

- `getnetworkinfo`
- `getconnectioncount`
- `getpeerinfo`

### Mempool

- `getmempoolinfo`

### Mining

- `getblocktemplate`
- `gettemplateinfo`
- `getmininginfo`

### Utility

- `validateathoaddress <address>`
- `sha3_384 <input>`

## Output Formats

- `pretty`: pretty-printed JSON, default
- `json`: compact JSON
- `table`: table view for array-of-object responses, with JSON fallback

## Safety Notes

- `atho-cli` submits commands through the node's existing validated RPC path.
- It does not mutate consensus state directly.
- Dangerous confirmation support is wired in with `--confirm`, but the current initial command set is read-focused.

## Current Limitation

The present local RPC transport is still local-only and does **not** yet implement cookie auth or username/password auth.

That is why these flags currently return a clear unsupported error:

- `--cookie-auth`
- `--rpc-user`
- `--rpc-password`
- `--timeout`

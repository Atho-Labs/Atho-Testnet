# Atho Production API Guide

## Scope

This guide documents the HTTP API that exists in the current Atho repository. It is intentionally conservative: it describes what is implemented today, what is gated behind config, and what is still missing for a public production deployment.

Base URL by default:

```text
http://127.0.0.1:8080
```

## Production Posture

- Default bind: `127.0.0.1`
- Default mode: read-only
- Default wallet/admin/mining HTTP routes: disabled
- Default CORS allowlist: `https://atho.io`, `https://www.atho.io`
- Default rate limit: `180` requests/minute
- Default heavy rate limit: `90` requests/minute
- Default max response size: `1,048,576` bytes

There is **no built-in authentication layer** in the HTTP API. For now, treat it as:

- safe for local use,
- safe for public read-only use only when fronted by external controls,
- not suitable for exposing privileged write operations directly to the internet.

## Environment Controls

| Variable | Purpose | Default |
| --- | --- | --- |
| `ATHO_API_ENABLED` | Enable or disable HTTP API | `true` |
| `ATHO_API_BIND` | Bind address | `127.0.0.1` |
| `ATHO_API_PORT` | TCP port | `8080` |
| `ATHO_API_PUBLIC_READ_ONLY` | Read-only profile flag | `true` |
| `ATHO_API_ADMIN_ENABLED` | Reserved admin flag | `false` |
| `ATHO_API_WALLET_ENABLED` | Enables raw transaction broadcast routes | `false` |
| `ATHO_API_MINING_ENABLED` | Reserved mining-write flag | `false` |
| `ATHO_API_ALLOWED_ORIGINS` | Comma-separated CORS allowlist | `atho.io` origins |
| `ATHO_API_RATE_LIMIT_ENABLED` | Enable request throttling | `true` |
| `ATHO_API_RATE_LIMIT_RPM` | Standard request rate | `180` |
| `ATHO_API_HEAVY_RATE_LIMIT_RPM` | Heavy endpoint rate | `90` |
| `ATHO_API_MAX_RESPONSE_BYTES` | Response cap | `1,048,576` |
| `ATHO_EXPLORER_INDEX_ENABLED` | Explorer index maintenance | `true` |
| `ATHO_EXPLORER_SNAPSHOT_ENABLED` | Explorer snapshot persistence | `true` |

## Error Model

Common JSON error `code` values:

- `not_found`
- `invalid_input`
- `method_not_allowed`
- `wallet_api_disabled`
- `origin_not_allowed`
- `rate_limited`
- `response_too_large`
- `explorer_index_not_ready`

Consensus-relevant raw transaction submissions may also surface mapped validation/RPC errors such as:

- noncanonical raw transaction rejection
- legacy lock format rejection
- wrong-network or invalid witness/signature rejection

## Authentication and Exposure

| Endpoint class | Auth in repo | Recommended exposure |
| --- | --- | --- |
| Public read endpoints | None | Reverse proxy + rate limit + TLS |
| Transaction broadcast endpoints | None, but disabled unless wallet API enabled | Local only |
| Admin endpoints | Not exposed via HTTP routes in current repo | Keep absent or authenticated externally |
| Wallet secret management | Not exposed via HTTP routes | Do not expose |

## Implemented Read Endpoints

All endpoints below are `GET` unless stated otherwise.

| Path | Purpose | Request parameters | Response summary | Auth | Rate limit class | Status |
| --- | --- | --- | --- | --- | --- | --- |
| `/api/v1/health` | Health/readiness snapshot | none | API online flag, network, sync/readiness labels | None | Standard | Implemented |
| `/api/v1/status` | Full status overview | none | Node status, API flags, sync and peer summaries | None | Standard | Implemented |
| `/api/v1/tip` | Chain tip summary | none | Tip hash, height, timestamp | None | Standard | Implemented |
| `/api/v1/blocks/latest?limit=N` | Latest block list | `limit` | Recent block summaries | None | Heavy | Implemented |
| `/api/v1/block/height/<height>` | Block by height | height path | Block detail | None | Heavy | Implemented |
| `/api/v1/block/hash/<hash>` | Block by hash | hash path | Block detail | None | Heavy | Implemented |
| `/api/v1/tx/<txid>` | Transaction detail | txid path | Raw tx/detail view | None | Heavy | Implemented |
| `/api/v1/address/<address>?limit=&offset=` | Address summary/history | address path, paging | Balance-ish summary and address activity | None | Heavy | Implemented |
| `/api/v1/address/<address>/utxos?limit=&offset=` | Address UTXOs | address path, paging | UTXO list | None | Heavy | Implemented |
| `/api/v1/mempool` | Full mempool listing view | paging-style query when supported | Mempool entries | None | Heavy | Implemented |
| `/api/v1/mempool/summary` | Lightweight mempool summary | none | Counts, totals | None | Standard | Implemented |
| `/api/v1/mempool/tx/<txid>` | Single mempool tx | txid path | Tx summary if present | None | Heavy | Implemented |
| `/api/v1/fees` | Fee summary | none | Fee recommendation surface | None | Standard | Implemented |
| `/api/v1/supply` | Supply and emission view | none | Supply totals | None | Standard | Implemented |
| `/api/v1/peers/summary` | Peer summary | none | Peer counts and health summary | None | Standard | Implemented |
| `/api/v1/network` | Network overview | none | Network name/id, API profile, sync profile | None | Standard | Implemented |
| `/api/v1/network/stats` | Aggregated network stats | none | Stats/difficulty/hashrate-like info | None | Heavy | Implemented |
| `/api/v1/network/hashrate` | Hashrate summary | none | Estimated hashrate | None | Standard | Implemented |
| `/api/v1/network/uptime` | Uptime summary | none | Uptime data | None | Standard | Implemented |
| `/api/v1/network/peers` | Peer list view | none | Peer details | None | Heavy | Implemented |
| `/api/v1/network/supply` | Supply alias route | none | Same supply family data | None | Standard | Implemented |
| `/api/v1/network/difficulty` | Difficulty summary | none | Difficulty metrics | None | Standard | Implemented |
| `/api/v1/network/blocktime` | Block interval summary | none | Blocktime metrics | None | Standard | Implemented |

## Implemented Write Endpoints

These routes are `POST` and only work when `ATHO_API_WALLET_ENABLED=1`.

| Path | Purpose | Request body | Response | Auth | Recommended exposure | Status |
| --- | --- | --- | --- | --- | --- | --- |
| `/api/v1/tx/broadcast` | Submit raw transaction | JSON with `raw_tx_hex` or accepted alias keys, or plain body hex | Broadcast result / rejection | None in repo | Local only | Implemented |
| `/api/v1/tx/sendraw` | Alias of broadcast | same | same | None in repo | Local only | Implemented |
| `/api/v1/sendrawtransaction` | Alias of broadcast | same | same | None in repo | Local only | Implemented |

### Broadcast request example

```http
POST /api/v1/tx/broadcast
Content-Type: application/json

{"raw_tx_hex":"<hex>"}
```

### Broadcast success example

```json
{
  "success": true,
  "network": "testnet",
  "network_id": "atho-testnet",
  "api_version": "v1",
  "data": {
    "accepted": true
  }
}
```

## Example Read Response

```json
{
  "success": true,
  "network": "regnet",
  "network_id": "atho-regnet",
  "api_version": "v1",
  "data": {
    "status": "Healthy"
  }
}
```

## Security Notes By Endpoint Family

### Health and status

- Safe to expose behind a reverse proxy.
- Avoid leaking internal topology details to untrusted public consumers unless needed.

### Block/tx/address explorer endpoints

- Heavy endpoints should stay paginated.
- Keep `ATHO_API_MAX_RESPONSE_BYTES` enabled.
- Use CDN/cache only for stable block data, not for mempool state.

### Raw transaction submission

- Must remain behind the mempool validation path.
- Never bypass canonical decode or witness validation.
- Keep this local-only unless authentication and abuse controls are added outside the node.

## Missing or Incomplete Production APIs

The following areas are implied by product goals but are **not** complete HTTP contracts in the current repo:

| Area | Current status | Note |
| --- | --- | --- |
| Authenticated admin HTTP API | Missing | Current repo intentionally does not expose it |
| Wallet secret-management HTTP API | Missing | Good for safety; keep it that way unless designed carefully |
| Public mining-control HTTP API | Missing/disabled | Mining flags exist but write routes are not a public contract |
| Metrics endpoint | Missing | Health/readiness exists, but there is no dedicated metrics surface |
| Formal versioned public API schema | Incomplete | Current routes are stable enough for local use, not yet a polished public contract |

## Recommended Production Deployment Policy

1. Keep the API bound to loopback by default.
2. Put any public read-only exposure behind:
   - TLS termination,
   - request limits,
   - response caching where appropriate,
   - IP and origin policy.
3. Keep transaction broadcast routes local-only unless an explicit public mempool gateway is intentionally built.
4. Do not expose wallet, seed, mnemonic, or private-key operations via this HTTP surface.

## Tests Covering The HTTP Surface

Representative current tests include:

- health route readiness test
- hidden write/admin route reachability tests
- invalid input routing tests
- mempool route behavior tests
- raw transaction submission acceptance/rejection tests
- legacy lock rejection via `sendrawtransaction`
- noncanonical raw transaction rejection via `sendrawtransaction`

## Production Readiness Verdict

For **local and controlled read-only use**, the API is in good shape.

For **public production exposure**, it is **not yet sufficient by itself** because:

- there is no built-in authentication layer,
- write surfaces rely on external operator discipline,
- and the public contract documentation is only now catching up with the code.

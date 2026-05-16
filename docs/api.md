# API

Atho includes a local HTTP API for node status, explorer, network, block, transaction, mempool, fee, supply, and peer information.

## Start The API

The API starts with `athod` by default.

```bash
cargo run -p atho-node --bin athod -- --network testnet
```

Default base URL:

```text
http://127.0.0.1:8080
```

Health check:

```bash
curl http://127.0.0.1:8080/api/v1/health
```

## Configuration

Useful environment variables:

- `ATHO_API_ENABLED`
- `ATHO_API_BIND`
- `ATHO_API_PORT`
- `ATHO_API_PUBLIC_READ_ONLY`
- `ATHO_API_ALLOWED_ORIGINS`
- `ATHO_API_RATE_LIMIT_ENABLED`
- `ATHO_API_RATE_LIMIT_RPM`
- `ATHO_API_HEAVY_RATE_LIMIT_RPM`
- `ATHO_API_MAX_RESPONSE_BYTES`

Defaults are local, read-only, rate-limited, and CORS-restricted.

## Read Endpoints

- `GET /api/v1/health`
- `GET /api/v1/status`
- `GET /api/v1/tip`
- `GET /api/v1/blocks/latest?limit=10`
- `GET /api/v1/block/height/<height>`
- `GET /api/v1/block/hash/<hash>`
- `GET /api/v1/tx/<txid>`
- `GET /api/v1/address/<address>?limit=25&offset=0`
- `GET /api/v1/address/<address>/utxos?limit=25&offset=0`
- `GET /api/v1/mempool`
- `GET /api/v1/mempool/summary`
- `GET /api/v1/mempool/tx/<txid>`
- `GET /api/v1/fees`
- `GET /api/v1/supply`
- `GET /api/v1/peers/summary`
- `GET /api/v1/network`
- `GET /api/v1/network/stats`
- `GET /api/v1/network/hashrate`
- `GET /api/v1/network/uptime`
- `GET /api/v1/network/peers`
- `GET /api/v1/network/supply`
- `GET /api/v1/network/difficulty`
- `GET /api/v1/network/blocktime`

## Transaction Broadcast

The API has transaction broadcast routes, but they are disabled by default unless `ATHO_API_WALLET_ENABLED=1`.

- `POST /api/v1/tx/broadcast`
- `POST /api/v1/tx/sendraw`
- `POST /api/v1/sendrawtransaction`

Example:

```bash
curl -X POST http://127.0.0.1:8080/api/v1/tx/broadcast \
  -H 'Content-Type: application/json' \
  -d '{"raw_tx_hex":"<hex>"}'
```

## Example Response

```json
{
  "success": true,
  "network": "testnet",
  "network_id": "atho-testnet",
  "api_version": "v1",
  "data": {}
}
```

## Common Errors

- `origin_not_allowed`: request origin is not in the CORS allowlist
- `method_not_allowed`: unsupported HTTP method or disabled write route
- `wallet_api_disabled`: transaction broadcast API is disabled
- `rate_limited`: request rate exceeded
- `response_too_large`: response exceeded `ATHO_API_MAX_RESPONSE_BYTES`
- `explorer_index_not_ready`: address/index endpoint was requested before the index was ready
- `invalid_input`: invalid hash, height, address, query, or request body
- `not_found`: route or object was not found

## Authentication

The HTTP API has no authentication layer in this repo. Keep it bound to loopback unless you are deliberately exposing a read-only public endpoint with external controls.

// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! HTTP API layer for explorer, node-status, and optional transaction broadcast use cases.
//!
//! The public surface here stays narrow by default: local bind, explicit CORS
//! allowlist, rate limits, and read-only routes unless wallet write support is
//! explicitly enabled. HTTP callers read through the existing node service
//! boundary rather than touching LMDB state directly.
use crate::config::ApiConfig;
use crate::dev;
use crate::mempool::MempoolEntry;
use crate::service::NodeService;
use atho_core::address::decode_base56_address;
use atho_core::consensus::{params::consensus_params_for_network, subsidy};
use atho_core::constants::{
    ATOMS_PER_ATHO, BLOCKS_PER_YEAR, MAX_STANDARD_INPUTS, MAX_STANDARD_OUTPUTS,
    MAX_TRANSACTION_RAW_BYTES, MIN_OUTPUT_AMOUNT_ATOMS, MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE,
    MIN_TX_FEE_ATOMS, TX_POW_MAX_BITS, TX_POW_MIN_BITS,
};
use atho_core::network::Network;
use atho_rpc::command::CommandInvocation;
use atho_rpc::error::RpcError;
use atho_rpc::request::RpcRequest;
use atho_rpc::response::RpcResponse;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

type SharedSystem = Arc<Mutex<crate::system::AthoSystem>>;
const DIFFICULTY_DISPLAY_SCALE: u64 = 100_000_000;
const MAX_QUERY_PARAMS: usize = 16;
const MAX_QUERY_KEY_BYTES: usize = 64;
const MAX_QUERY_VALUE_BYTES: usize = 256;
const MAX_TX_BROADCAST_BODY_BYTES: usize = MAX_TRANSACTION_RAW_BYTES * 2 + 4096;

pub fn bind_http_server(config: &ApiConfig) -> Result<Server, String> {
    Server::http(config.bind_address()).map_err(|err| err.to_string())
}

pub fn run_http_server(
    server: Server,
    shared: SharedSystem,
    config: ApiConfig,
) -> Result<(), String> {
    let bind = config.bind_address();
    let state = HttpApiState {
        shared,
        config,
        rate_limiter: Mutex::new(RateLimiter::default()),
    };
    let _ = dev::append_log("api", &format!("http api listening on {bind}"));

    for request in server.incoming_requests() {
        state.handle_request(request);
    }
    Ok(())
}

struct HttpApiState {
    shared: SharedSystem,
    config: ApiConfig,
    rate_limiter: Mutex<RateLimiter>,
}

#[derive(Debug, Default)]
struct RateLimiter {
    global: BTreeMap<String, VecDeque<u64>>,
    heavy: BTreeMap<String, VecDeque<u64>>,
}

#[derive(Debug, Clone)]
struct ApiReply {
    status: u16,
    body: Value,
    allow_origin: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointClass {
    Standard,
    Heavy,
}

#[derive(Debug, Clone)]
struct ApiError {
    status: u16,
    code: &'static str,
}

#[derive(Debug, Clone)]
struct SupplySnapshot {
    height: u64,
    total_mined_supply_atoms: u128,
    circulating_supply_atoms: u128,
    burned_supply_atoms: u128,
    max_supply_atoms: Option<u128>,
    current_block_reward_atoms: u64,
    next_halving_height: Option<u64>,
    blocks_until_halving: Option<u64>,
    emission_epoch: u64,
    coinbase_maturity_blocks: u64,
}

impl HttpApiState {
    fn handle_request(&self, mut request: Request) {
        let method = request.method().clone();
        let url = request.url().to_string();
        let remote_ip = request
            .remote_addr()
            .map(|addr| addr.ip().to_string())
            .unwrap_or_else(|| String::from("unknown"));
        let origin = header_value(&request, "Origin");
        let allow_origin = origin
            .as_deref()
            .and_then(|origin| self.resolve_allowed_origin(origin));

        let (path, query) = split_url(&url);
        let body = if method == Method::Post {
            match read_limited_body(&mut request, MAX_TX_BROADCAST_BODY_BYTES) {
                Ok(body) => Some(body),
                Err(error) => {
                    let reply = self.error_reply(self.config.explorer.network, error, allow_origin);
                    return self.respond(request, reply, &remote_ip, &method, &path);
                }
            }
        } else {
            None
        };
        let reply = match self.dispatch_with_body(
            &method,
            &path,
            &query,
            body.as_deref(),
            &remote_ip,
            origin.is_some(),
            allow_origin.clone(),
        ) {
            Ok(reply) => reply,
            Err(error) => self.error_reply(self.config.explorer.network, error, allow_origin),
        };

        self.respond(request, reply, &remote_ip, &method, &path);
    }

    fn respond(
        &self,
        request: Request,
        reply: ApiReply,
        remote_ip: &str,
        method: &Method,
        path: &str,
    ) {
        let body = match serde_json::to_vec(&reply.body) {
            Ok(body) => body,
            Err(_) => serde_json::to_vec(&api_error_body(
                self.config.explorer.network,
                "internal_error",
            ))
            .unwrap_or_else(|_| b"{\"success\":false,\"error\":\"internal_error\"}".to_vec()),
        };

        let mut response = Response::from_data(body).with_status_code(StatusCode(reply.status));
        if let Ok(header) = Header::from_bytes("Content-Type", "application/json; charset=utf-8") {
            response = response.with_header(header);
        }
        if let Ok(header) = Header::from_bytes("Cache-Control", "no-store") {
            response = response.with_header(header);
        }
        if let Some(origin) = reply.allow_origin {
            if let Ok(header) = Header::from_bytes("Access-Control-Allow-Origin", origin) {
                response = response.with_header(header);
            }
            if let Ok(header) = Header::from_bytes("Vary", "Origin") {
                response = response.with_header(header);
            }
            let methods = if self.config.wallet_enabled {
                "GET, POST, OPTIONS"
            } else {
                "GET, OPTIONS"
            };
            if let Ok(header) = Header::from_bytes("Access-Control-Allow-Methods", methods) {
                response = response.with_header(header);
            }
            if let Ok(header) = Header::from_bytes("Access-Control-Allow-Headers", "Content-Type") {
                response = response.with_header(header);
            }
        }

        let _ = request.respond(response);
        let _ = dev::append_log(
            "api",
            &format!("{remote_ip} {method:?} {} {}", path, reply.status),
        );
    }

    #[cfg(test)]
    fn dispatch(
        &self,
        method: &Method,
        path: &str,
        query: &BTreeMap<String, String>,
        remote_ip: &str,
        origin_present: bool,
        allow_origin: Option<String>,
    ) -> Result<ApiReply, ApiError> {
        self.dispatch_with_body(
            method,
            path,
            query,
            None,
            remote_ip,
            origin_present,
            allow_origin,
        )
    }

    fn dispatch_with_body(
        &self,
        method: &Method,
        path: &str,
        query: &BTreeMap<String, String>,
        body: Option<&str>,
        remote_ip: &str,
        origin_present: bool,
        allow_origin: Option<String>,
    ) -> Result<ApiReply, ApiError> {
        if origin_present && allow_origin.is_none() {
            return Err(ApiError {
                status: 403,
                code: "origin_not_allowed",
            });
        }

        if method == &Method::Options {
            return allow_origin
                .map(|origin| ApiReply {
                    status: 204,
                    body: api_identity_body(self.config.explorer.network),
                    allow_origin: Some(origin),
                })
                .ok_or(ApiError {
                    status: 403,
                    code: "origin_not_allowed",
                });
        }

        if method != &Method::Get && method != &Method::Post {
            return Err(ApiError {
                status: 405,
                code: "method_not_allowed",
            });
        }
        if method == &Method::Post && !is_transaction_broadcast_path(path) {
            return Err(ApiError {
                status: 405,
                code: "method_not_allowed",
            });
        }

        validate_path(path)?;
        validate_query(query)?;

        let endpoint_class = classify_endpoint(path);
        self.enforce_rate_limit(remote_ip, endpoint_class)?;

        let mut service = self
            .shared
            .lock()
            .expect("http api node service mutex poisoned");
        let network = service.network();
        if method == &Method::Get && !self.config.public_read_only {
            return Err(ApiError {
                status: 403,
                code: "public_read_only_required",
            });
        }

        let data = if method == &Method::Post {
            match route_post_request(
                &self.config,
                &mut service,
                network,
                path,
                query,
                body.unwrap_or_default(),
            ) {
                Ok(data) => data,
                Err(error) => {
                    let _ = dev::append_log(
                        "api",
                        &format!(
                            "request failed network={} path={} error={}",
                            network.domain_tag(),
                            path,
                            error.code
                        ),
                    );
                    return Err(error);
                }
            }
        } else {
            match route_request(&self.config, &mut service, network, path, query) {
                Ok(data) => data,
                Err(error) => {
                    let _ = dev::append_log(
                        "api",
                        &format!(
                            "request failed network={} path={} error={}",
                            network.domain_tag(),
                            path,
                            error.code
                        ),
                    );
                    return Err(error);
                }
            }
        };

        let body = json!({
            "success": true,
            "network": network.domain_tag(),
            "network_id": network.id(),
            "network_name": network.domain_tag(),
            "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
            "api_version": "v1",
            "node_version": env!("CARGO_PKG_VERSION"),
            "data": data,
        });
        let bytes = serde_json::to_vec(&body).map_err(|_| ApiError {
            status: 500,
            code: "internal_error",
        })?;
        if bytes.len() > self.config.max_response_bytes {
            return Err(ApiError {
                status: 413,
                code: "response_too_large",
            });
        }

        Ok(ApiReply {
            status: 200,
            body,
            allow_origin,
        })
    }

    fn resolve_allowed_origin(&self, origin: &str) -> Option<String> {
        self.config
            .cors
            .allowed_origins
            .iter()
            .find(|allowed| allowed.as_str() == origin)
            .cloned()
    }

    fn enforce_rate_limit(
        &self,
        remote_ip: &str,
        endpoint_class: EndpointClass,
    ) -> Result<(), ApiError> {
        if !self.config.rate_limit.enabled {
            return Ok(());
        }
        let now = unix_timestamp();
        let mut limiter = self
            .rate_limiter
            .lock()
            .expect("http api rate limiter mutex poisoned");
        if !allow_rate_limit(
            limiter.global.entry(remote_ip.to_string()).or_default(),
            self.config.rate_limit.requests_per_minute as usize,
            now,
        ) {
            return Err(ApiError {
                status: 429,
                code: "rate_limited",
            });
        }
        if endpoint_class == EndpointClass::Heavy
            && !allow_rate_limit(
                limiter.heavy.entry(remote_ip.to_string()).or_default(),
                self.config.rate_limit.heavy_requests_per_minute as usize,
                now,
            )
        {
            return Err(ApiError {
                status: 429,
                code: "rate_limited",
            });
        }
        Ok(())
    }

    fn error_reply(
        &self,
        network: Network,
        error: ApiError,
        allow_origin: Option<String>,
    ) -> ApiReply {
        ApiReply {
            status: error.status,
            body: api_error_body(network, error.code),
            allow_origin,
        }
    }
}

fn api_identity_body(network: Network) -> Value {
    json!({
        "success": true,
        "network": network.domain_tag(),
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
    })
}

fn api_error_body(network: Network, error: &str) -> Value {
    let mut body = api_identity_body(network);
    if let Value::Object(ref mut object) = body {
        object.insert(String::from("success"), Value::Bool(false));
        object.insert(String::from("error"), Value::String(error.to_string()));
    }
    body
}

fn route_request(
    config: &ApiConfig,
    service: &mut NodeService,
    network: Network,
    path: &str,
    query: &BTreeMap<String, String>,
) -> Result<Value, ApiError> {
    let parts = path
        .trim_start_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 || parts[0] != "api" || parts[1] != "v1" {
        return Err(ApiError {
            status: 404,
            code: "not_found",
        });
    }

    if endpoint_requires_explorer_refresh(&parts) {
        service.refresh_api_views();
    } else {
        service.refresh_api_light_views();
    }

    match parts.get(2).copied() {
        Some("health") if parts.len() == 3 => health_value(service),
        Some("status") if parts.len() == 3 => status_value(config, service),
        Some("tip") if parts.len() == 3 => tip_value(service),
        Some("blocks") if parts.get(3) == Some(&"latest") && parts.len() == 4 => {
            latest_blocks_value(service, query)
        }
        Some("block") if parts.get(3) == Some(&"height") && parts.len() == 5 => {
            command_value(service, "getblock", vec![parts[4].to_string()])
        }
        Some("block") if parts.get(3) == Some(&"hash") && parts.len() == 5 => {
            command_value(service, "getblock", vec![parts[4].to_string()])
        }
        Some("tx") if parts.len() == 4 => {
            command_value(service, "getrawtransaction", vec![parts[3].to_string()])
        }
        Some("address") if parts.len() == 4 => {
            address_summary_value(service, network, parts[3], query)
        }
        Some("address") if parts.len() == 5 && parts[4] == "utxos" => {
            address_utxos_value(service, network, parts[3], query)
        }
        Some("mempool") if parts.len() == 5 && parts[3] == "tx" => {
            mempool_tx_value(service, network, parts[4])
        }
        Some("mempool") if parts.len() == 4 && parts[3] == "summary" => {
            mempool_summary_value(service)
        }
        Some("mempool") if parts.len() == 3 => mempool_value(service, query),
        Some("fees") if parts.len() == 3 => fees_value(service, network),
        Some("supply") if parts.len() == 3 => supply_value(service, network),
        Some("peers") if parts.len() == 4 && parts[3] == "summary" => peers_summary_value(service),
        Some("network") if parts.len() == 4 && parts[3] == "stats" => network_stats_value(service),
        Some("network") if parts.len() == 4 && parts[3] == "hashrate" => {
            network_hashrate_value(service)
        }
        Some("network") if parts.len() == 4 && parts[3] == "uptime" => {
            network_uptime_value(service)
        }
        Some("network") if parts.len() == 4 && parts[3] == "peers" => network_peers_value(service),
        Some("network") if parts.len() == 4 && parts[3] == "supply" => {
            supply_value(service, network)
        }
        Some("network") if parts.len() == 4 && parts[3] == "difficulty" => {
            network_difficulty_value(service)
        }
        Some("network") if parts.len() == 4 && parts[3] == "blocktime" => {
            network_blocktime_value(service)
        }
        Some("network") if parts.len() == 3 => network_value(config, service),
        _ => Err(ApiError {
            status: 404,
            code: "not_found",
        }),
    }
}

fn route_post_request(
    config: &ApiConfig,
    service: &mut NodeService,
    _network: Network,
    path: &str,
    query: &BTreeMap<String, String>,
    body: &str,
) -> Result<Value, ApiError> {
    if !config.wallet_enabled {
        return Err(ApiError {
            status: 403,
            code: "wallet_api_disabled",
        });
    }
    if !query.is_empty() {
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }

    match path {
        "/api/v1/tx/broadcast" | "/api/v1/tx/sendraw" | "/api/v1/sendrawtransaction" => {
            let raw_tx_hex = parse_raw_transaction_request_body(body)?;
            service
                .broadcast_raw_transaction_hex_value(&raw_tx_hex)
                .map_err(map_broadcast_rpc_error)
        }
        _ => Err(ApiError {
            status: 405,
            code: "method_not_allowed",
        }),
    }
}

fn is_transaction_broadcast_path(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/tx/broadcast" | "/api/v1/tx/sendraw" | "/api/v1/sendrawtransaction"
    )
}

fn parse_raw_transaction_request_body(body: &str) -> Result<String, ApiError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }
    if trimmed.starts_with('{') {
        let value: Value = serde_json::from_str(trimmed).map_err(|_| ApiError {
            status: 400,
            code: "invalid_input",
        })?;
        for key in [
            "raw_tx_hex",
            "transaction_hex",
            "tx_hex",
            "raw_transaction_hex",
        ] {
            if let Some(raw) = value.get(key).and_then(Value::as_str) {
                if raw.trim().is_empty() {
                    break;
                }
                return Ok(raw.to_string());
            }
        }
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }
    Ok(trimmed.to_string())
}

fn map_broadcast_rpc_error(error: RpcError) -> ApiError {
    let lowered = error
        .details
        .as_deref()
        .unwrap_or(error.message.as_str())
        .to_ascii_lowercase();
    if lowered.contains("transaction submission is paused") {
        return ApiError {
            status: 503,
            code: "node_not_synced",
        };
    }
    ApiError {
        status: 400,
        code: "rejected_transaction",
    }
}

fn status_value(config: &ApiConfig, service: &NodeService) -> Result<Value, ApiError> {
    let status = service.node_status();
    let validation_safe = status
        .network_diagnostics
        .chain_validation_status
        .is_empty()
        || status.network_diagnostics.safe_to_serve;
    let chain_synced = status.running
        && status.headers_synced
        && status.block_count >= status.sync_best_height
        && validation_safe;
    let index_tip_hash = service.explorer_index().tip_hash();
    Ok(json!({
        "api_online": true,
        "api_version": "v1",
        "network_id": status.network.id(),
        "network_name": status.network.domain_tag(),
        "node_version": env!("CARGO_PKG_VERSION"),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(status.network)),
        "running": status.running,
        "headers_synced": status.headers_synced,
        "chain_synced": chain_synced,
        "height": status.block_count,
        "chain_height": status.block_count,
        "tip_hash": hex::encode(status.tip_hash),
        "tip_timestamp": status.tip_timestamp,
        "mempool_count": status.mempool_count,
        "peer_count": status.network_diagnostics.peer_count,
        "sync_best_height": status.sync_best_height,
        "sync_mode": status.network_diagnostics.sync_mode.clone(),
        "chain_validation_status": status.network_diagnostics.chain_validation_status.clone(),
        "safe_to_mine": status.network_diagnostics.safe_to_mine,
        "safe_to_serve": status.network_diagnostics.safe_to_serve,
        "validation_lag_blocks": status.network_diagnostics.validation_lag_blocks,
        "pending_validation_blocks": status.network_diagnostics.pending_validation_blocks,
        "untrusted_downloaded_blocks": status.network_diagnostics.untrusted_downloaded_blocks,
        "index": {
            "enabled": service.explorer_index_enabled(),
            "ready": service.explorer_index_ready(),
            "height": service.explorer_index().tip_height(),
            "tip_hash": hex::encode(index_tip_hash),
            "source": service.explorer_index_source(),
            "snapshot_enabled": service.explorer_snapshot_enabled(),
            "snapshot_persisted_at": service.explorer_snapshot_persisted_unix(),
        },
        "api": {
            "public_read_only": config.public_read_only,
            "wallet_enabled": config.wallet_enabled,
            "mining_enabled": config.mining_enabled,
            "admin_enabled": config.admin_enabled,
            "max_response_bytes": config.max_response_bytes,
        }
    }))
}

fn health_value(service: &NodeService) -> Result<Value, ApiError> {
    let status = service.node_status();
    let validation_safe = status
        .network_diagnostics
        .chain_validation_status
        .is_empty()
        || status.network_diagnostics.safe_to_serve;
    let chain_synced = status.running
        && status.headers_synced
        && status.block_count >= status.sync_best_height
        && validation_safe;
    let health = network_health_label(&status);
    Ok(json!({
        "api_online": true,
        "network_id": status.network.id(),
        "network_name": status.network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(status.network)),
        "node_version": env!("CARGO_PKG_VERSION"),
        "running": status.running,
        "headers_synced": status.headers_synced,
        "chain_synced": chain_synced,
        "syncing": !chain_synced,
        "chain_height": status.block_count,
        "sync_best_height": status.sync_best_height,
        "peer_count": status.network_diagnostics.peer_count,
        "mempool_count": status.mempool_count,
        "sync_mode": status.network_diagnostics.sync_mode.clone(),
        "chain_validation_status": status.network_diagnostics.chain_validation_status.clone(),
        "safe_to_mine": status.network_diagnostics.safe_to_mine,
        "validation_lag_blocks": status.network_diagnostics.validation_lag_blocks,
        "index_ready": service.explorer_index_ready(),
        "index_source": service.explorer_index_source(),
        "status": health,
    }))
}

fn tip_value(service: &NodeService) -> Result<Value, ApiError> {
    let status = service.node_status();
    Ok(json!({
        "network_id": status.network.id(),
        "network_name": status.network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(status.network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "height": status.block_count,
        "hash": hex::encode(status.tip_hash),
        "timestamp": status.tip_timestamp,
    }))
}

fn latest_blocks_value(
    service: &NodeService,
    query: &BTreeMap<String, String>,
) -> Result<Value, ApiError> {
    let limit = parse_limit(query, 10, 50)?;
    let status = service.node_status();
    let mut blocks = Vec::new();
    let mut next_height = status.block_count;
    while blocks.len() < limit {
        let Some(record) = service.node_ref().block_record_by_height(next_height) else {
            break;
        };
        blocks.push(render_block_summary(&record));
        if next_height == 0 {
            break;
        }
        next_height = next_height.saturating_sub(1);
    }
    Ok(json!({
        "network_id": status.network.id(),
        "network_name": status.network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(status.network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "count": blocks.len(),
        "blocks": blocks,
    }))
}

fn address_summary_value(
    service: &NodeService,
    network: Network,
    address: &str,
    query: &BTreeMap<String, String>,
) -> Result<Value, ApiError> {
    validate_address_for_network(address, network)?;
    ensure_explorer_index_available(service)?;
    let limit = parse_limit(query, 25, 100)?;
    let offset = parse_offset(query)?;
    service
        .explorer_index()
        .address_summary_value(network, address, limit, offset)
        .ok_or(ApiError {
            status: 404,
            code: "not_found",
        })
}

fn address_utxos_value(
    service: &NodeService,
    network: Network,
    address: &str,
    query: &BTreeMap<String, String>,
) -> Result<Value, ApiError> {
    validate_address_for_network(address, network)?;
    ensure_explorer_index_available(service)?;
    let limit = parse_limit(query, 25, 100)?;
    let offset = parse_offset(query)?;
    service
        .explorer_index()
        .address_utxos_value(network, service.node_ref().height(), address, limit, offset)
        .ok_or(ApiError {
            status: 404,
            code: "not_found",
        })
}

fn mempool_value(
    service: &NodeService,
    query: &BTreeMap<String, String>,
) -> Result<Value, ApiError> {
    let limit = parse_limit(query, 25, 100)?;
    let offset = parse_offset(query)?;
    let mut entries = service
        .node_ref()
        .mempool_entries_iter()
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .received_at_unix()
            .cmp(&left.received_at_unix())
            .then(right.fee_atoms.cmp(&left.fee_atoms))
            .then(left.txid().cmp(&right.txid()))
    });
    let total = entries.len();
    let selected = paginate(&entries, limit, offset);
    let items = selected
        .iter()
        .map(|entry| {
            json!({
                "txid": hex::encode(entry.txid()),
                "wtxid": hex::encode(entry.wtxid()),
                "fee_atoms": entry.fee_atoms,
                "fee_atho": format_atoms_decimal(entry.fee_atoms),
                "size_bytes": entry.full_size_bytes(),
                "vsize_bytes": entry.vsize_bytes(),
                "feerate_atoms_per_vbyte": entry.feerate_atoms_per_vbyte(),
                "received_at": entry.received_at_unix(),
            })
        })
        .collect::<Vec<_>>();
    let summary = service.mempool_summary();
    Ok(json!({
        "transaction_count": total,
        "mempool_size_bytes": summary.mempool_size_bytes,
        "mempool_vsize_bytes": summary.mempool_vsize_bytes,
        "total_fee_atoms": summary.total_fee_atoms,
        "average_fee_atoms": summary.average_fee_atoms,
        "average_fee": format!("{} ATHO", format_atoms_decimal(summary.average_fee_atoms)),
        "highest_fee": summary.highest_fee.as_ref().map(render_cached_pending_transaction_summary),
        "lowest_fee": summary.lowest_fee.as_ref().map(render_cached_pending_transaction_summary),
        "estimated_next_block_tx_count": summary.estimated_next_block_tx_count,
        "status": summary.status,
        "fingerprint": hex::encode(service.node_ref().mempool_fingerprint()),
        "transactions": items,
        "page": {
            "limit": limit,
            "offset": offset,
            "returned": items.len(),
            "total": total,
        }
    }))
}

fn mempool_summary_value(service: &NodeService) -> Result<Value, ApiError> {
    let summary = service.mempool_summary();
    Ok(json!({
        "pending_transactions": summary.transaction_count,
        "mempool_size_bytes": summary.mempool_size_bytes,
        "mempool_vsize_bytes": summary.mempool_vsize_bytes,
        "total_fee_atoms": summary.total_fee_atoms,
        "average_fee_atoms": summary.average_fee_atoms,
        "average_fee": format!("{} ATHO", format_atoms_decimal(summary.average_fee_atoms)),
        "highest_fee": summary.highest_fee.as_ref().map(render_cached_pending_transaction_summary),
        "lowest_fee": summary.lowest_fee.as_ref().map(render_cached_pending_transaction_summary),
        "estimated_next_block_tx_count": summary.estimated_next_block_tx_count,
        "status": summary.status,
        "recent_transactions": summary
            .recent_transactions
            .iter()
            .map(render_cached_pending_transaction_summary)
            .collect::<Vec<_>>(),
    }))
}

fn mempool_tx_value(
    service: &NodeService,
    network: Network,
    txid_hex: &str,
) -> Result<Value, ApiError> {
    let txid = parse_hash48_value(txid_hex)?;
    let entry = service.node_ref().mempool_entry(&txid).ok_or(ApiError {
        status: 404,
        code: "not_found",
    })?;
    let depends = service
        .node_ref()
        .mempool_dependency_txids(&txid)
        .unwrap_or_default()
        .into_iter()
        .map(hex::encode)
        .collect::<Vec<_>>();
    let descendants = service
        .node_ref()
        .mempool_descendant_txids(&txid)
        .unwrap_or_default()
        .into_iter()
        .map(hex::encode)
        .collect::<Vec<_>>();
    let transaction = command_value(service, "getrawtransaction", vec![txid_hex.to_string()])?
        .get("transaction")
        .cloned()
        .ok_or(ApiError {
            status: 500,
            code: "internal_error",
        })?;
    Ok(json!({
        "source": "mempool",
        "txid": hex::encode(entry.txid()),
        "fee_atoms": entry.fee_atoms,
        "fee_atho": format_atoms_decimal(entry.fee_atoms),
        "received_at_unix": entry.received_at_unix(),
        "entry": render_mempool_entry_value(network, &entry, &depends, &descendants),
        "transaction": transaction,
    }))
}

fn fees_value(service: &NodeService, network: Network) -> Result<Value, ApiError> {
    let mut feerates = service
        .node_ref()
        .mempool_entries_iter()
        .map(|entry| entry.feerate_atoms_per_vbyte())
        .collect::<Vec<_>>();
    feerates.sort_unstable();
    let median = if feerates.is_empty() {
        MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE
    } else {
        feerates[feerates.len() / 2]
    };
    let min = feerates
        .first()
        .copied()
        .unwrap_or(MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE);
    let max = feerates
        .last()
        .copied()
        .unwrap_or(MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE);
    Ok(json!({
        "network": network.domain_tag(),
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "minimum_tx_fee_atoms": MIN_TX_FEE_ATOMS,
        "minimum_relay_fee_rate_atoms_per_vbyte": MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE,
        "minimum_output_amount_atoms": MIN_OUTPUT_AMOUNT_ATOMS,
        "max_standard_inputs": MAX_STANDARD_INPUTS,
        "max_standard_outputs": MAX_STANDARD_OUTPUTS,
        "transaction_pow": {
            "hash": "SHA3-256",
            "min_bits": TX_POW_MIN_BITS,
            "max_bits": TX_POW_MAX_BITS,
        },
        "mempool": {
            "minimum_feerate_atoms_per_vbyte": min,
            "median_feerate_atoms_per_vbyte": median,
            "maximum_feerate_atoms_per_vbyte": max,
        },
        "examples": [
            {"tx_vbytes": 250, "required_fee_atoms": MIN_TX_FEE_ATOMS.max(250 * MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE)},
            {"tx_vbytes": 500, "required_fee_atoms": MIN_TX_FEE_ATOMS.max(500 * MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE)},
            {"tx_vbytes": 650, "required_fee_atoms": MIN_TX_FEE_ATOMS.max(650 * MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE)},
            {"tx_vbytes": 1000, "required_fee_atoms": MIN_TX_FEE_ATOMS.max(1000 * MIN_RELAY_FEE_RATE_ATOMS_PER_VBYTE)},
        ]
    }))
}

fn supply_value(service: &NodeService, network: Network) -> Result<Value, ApiError> {
    let supply = supply_snapshot(service, network);
    let tail_height = subsidy::TAIL_EMISSION_START_HEIGHT;
    let annual_tail_atoms = subsidy::EMISSION_SCHEDULE.blocks_per_year as u128
        * subsidy::EMISSION_SCHEDULE.tail_reward_atoms as u128;
    Ok(json!({
        "height": supply.height,
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "total_mined_supply_atoms": supply.total_mined_supply_atoms.to_string(),
        "total_mined_supply": format!("{} ATHO", format_u128_atoms_decimal(supply.total_mined_supply_atoms)),
        "circulating_supply_atoms": supply.circulating_supply_atoms.to_string(),
        "circulating_supply": format!("{} ATHO", format_u128_atoms_decimal(supply.circulating_supply_atoms)),
        "burned_supply_atoms": supply.burned_supply_atoms.to_string(),
        "burned_supply": format!("{} ATHO", format_u128_atoms_decimal(supply.burned_supply_atoms)),
        "max_supply_atoms": supply.max_supply_atoms.map(|value| value.to_string()),
        "max_supply": supply.max_supply_atoms.map(|value| format!("{} ATHO", format_u128_atoms_decimal(value))),
        "max_supply_label": supply.max_supply_atoms.map(|value| format!("{} ATHO", format_u128_atoms_decimal(value))).unwrap_or_else(|| String::from("No Fixed Cap")),
        "current_block_reward_atoms": supply.current_block_reward_atoms,
        "current_block_reward": format!("{} ATHO", format_atoms_decimal(supply.current_block_reward_atoms)),
        "next_halving_height": supply.next_halving_height,
        "blocks_until_halving": supply.blocks_until_halving,
        "emission_epoch": supply.emission_epoch,
        "coinbase_maturity_blocks": supply.coinbase_maturity_blocks,
        "initial_block_reward_atoms": subsidy::EMISSION_SCHEDULE.initial_block_reward_atoms,
        "tail_reward_atoms": subsidy::EMISSION_SCHEDULE.tail_reward_atoms,
        "tail_emission_start_height": tail_height,
        "blocks_per_year": BLOCKS_PER_YEAR,
        "annual_tail_issuance_atoms": annual_tail_atoms.to_string(),
        "annual_tail_issuance_atho": format_u128_atoms_decimal(annual_tail_atoms),
    }))
}

fn peers_summary_value(service: &NodeService) -> Result<Value, ApiError> {
    let diagnostics = service.network_diagnostics();
    let network = service.network();
    let known_peer_addresses = known_peer_addresses_value(service, 128)?;
    Ok(json!({
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "peer_count": diagnostics.peer_count,
        "inbound_peer_count": diagnostics.inbound_peer_count,
        "outbound_peer_count": diagnostics.outbound_peer_count,
        "full_relay_peer_count": diagnostics.full_relay_peer_count,
        "block_relay_peer_count": diagnostics.block_relay_peer_count,
        "sync_peer_count": diagnostics.sync_peer_count,
        "tx_relay_peer_count": diagnostics.tx_relay_peer_count,
        "addr_relay_peer_count": diagnostics.addr_relay_peer_count,
        "topology_health_score": diagnostics.topology_health_score,
        "topology_warnings": diagnostics.topology_warnings.clone(),
        "sync_mode": diagnostics.sync_mode.clone(),
        "chain_validation_status": diagnostics.chain_validation_status.clone(),
        "best_header_height": diagnostics.best_header_height,
        "best_downloaded_body_height": diagnostics.best_downloaded_body_height,
        "best_validated_height": diagnostics.best_validated_height,
        "best_connected_height": diagnostics.best_connected_height,
        "latest_finalized_height": diagnostics.latest_finalized_height,
        "latest_finalized_hash": hex::encode(diagnostics.latest_finalized_hash),
        "pending_validation_blocks": diagnostics.pending_validation_blocks,
        "untrusted_downloaded_blocks": diagnostics.untrusted_downloaded_blocks,
        "untrusted_downloaded_bytes": diagnostics.untrusted_downloaded_bytes,
        "fast_download_enabled": diagnostics.fast_download_enabled,
        "checkpoint_anchored_sync_enabled": diagnostics.checkpoint_anchored_sync_enabled,
        "background_validation_enabled": diagnostics.background_validation_enabled,
        "safe_to_mine": diagnostics.safe_to_mine,
        "safe_to_serve": diagnostics.safe_to_serve,
        "validation_lag_blocks": diagnostics.validation_lag_blocks,
        "connecting_peer_count": diagnostics.connecting_peer_count,
        "known_peer_addresses": known_peer_addresses,
        "bytes_sent": diagnostics.bytes_sent,
        "bytes_received": diagnostics.bytes_received,
        "peers": diagnostics.peers.iter().map(|peer| json!({
            "remote_addr": peer.remote_addr,
            "direction": peer.direction,
            "roles": peer.roles.clone(),
            "handshake_ready": peer.handshake_ready,
            "best_height": peer.best_height,
            "protocol_version": peer.protocol_version,
            "services": peer.services,
            "quality_score": peer.quality_score,
        })).collect::<Vec<_>>(),
    }))
}

fn network_stats_value(service: &NodeService) -> Result<Value, ApiError> {
    let chain = service.chain_stats();
    let mempool = service.mempool_summary();
    let status = service.node_status();
    let diagnostics = &status.network_diagnostics;
    let known_nodes = service.known_node_count();
    let uptime_seconds = unix_timestamp().saturating_sub(chain.genesis_timestamp);
    let supply = supply_snapshot(service, status.network);
    let genesis_hash = hex::encode(atho_core::genesis::genesis_hash(status.network));
    let latest_block_hash = hex::encode(chain.tip_hash);
    let estimated_hashrate = format_hashrate(chain.estimated_hashrate_hps);
    let difficulty = chain.difficulty_ratio_scaled as f64 / DIFFICULTY_DISPLAY_SCALE as f64;
    let difficulty_display = format_scaled_decimal(chain.difficulty_ratio_scaled, 8);
    let block_reward = format!(
        "{} ATHO",
        format_atoms_decimal(chain.current_block_reward_atoms)
    );
    let current_block_reward = format!(
        "{} ATHO",
        format_atoms_decimal(supply.current_block_reward_atoms)
    );
    let total_mined_supply = format!(
        "{} ATHO",
        format_u128_atoms_decimal(supply.total_mined_supply_atoms)
    );
    let average_block_time = format_duration_seconds(chain.average_block_time_millis);
    let network_uptime = format_uptime(uptime_seconds);
    let circulating_supply = format!(
        "{} ATHO",
        format_u128_atoms_decimal(chain.circulating_supply_atoms)
    );
    let burned_supply = format!(
        "{} ATHO",
        format_u128_atoms_decimal(supply.burned_supply_atoms)
    );
    let max_supply = supply
        .max_supply_atoms
        .map(|value| format!("{} ATHO", format_u128_atoms_decimal(value)));
    let max_supply_atoms = supply.max_supply_atoms.map(|value| value.to_string());
    let max_supply_label = max_supply
        .clone()
        .unwrap_or_else(|| String::from("No Fixed Cap"));
    let average_fee = format!(
        "{} ATHO",
        format_atoms_decimal(chain.average_confirmed_fee_atoms)
    );
    let status_label = network_health_label(&status);
    let index_source = service.explorer_index_source();
    let window = json!({
        "hashrate_blocks": chain.hashrate_window_blocks,
        "blocktime_blocks": chain.blocktime_window_blocks,
        "fee_transactions": chain.average_fee_window_transactions,
        "fee_blocks": chain.average_fee_window_blocks,
    });
    Ok(json!({
        "api_version": "v1",
        "network_id": status.network.id(),
        "network_name": status.network.domain_tag(),
        "node_version": env!("CARGO_PKG_VERSION"),
        "genesis_hash": genesis_hash,
        "height": chain.tip_height,
        "latest_block_hash": latest_block_hash,
        "estimated_hashrate_hps": chain.estimated_hashrate_hps,
        "estimated_hashrate": estimated_hashrate,
        "active_peers": diagnostics.peer_count,
        "known_nodes": known_nodes,
        "total_transactions": chain.total_transactions,
        "total_blocks": chain.total_blocks,
        "difficulty": difficulty,
        "difficulty_display": difficulty_display,
        "block_reward_atoms": chain.current_block_reward_atoms,
        "block_reward": block_reward,
        "current_block_reward_atoms": supply.current_block_reward_atoms,
        "current_block_reward": current_block_reward,
        "total_mined_supply_atoms": supply.total_mined_supply_atoms.to_string(),
        "total_mined_supply": total_mined_supply,
        "average_block_time_seconds": chain.average_block_time_millis as f64 / 1_000.0,
        "average_block_time": average_block_time,
        "network_uptime_seconds": uptime_seconds,
        "network_uptime": network_uptime,
        "circulating_supply_atoms": chain.circulating_supply_atoms.to_string(),
        "circulating_supply": circulating_supply,
        "burned_supply_atoms": supply.burned_supply_atoms.to_string(),
        "burned_supply": burned_supply,
        "max_supply_atoms": max_supply_atoms,
        "max_supply": max_supply,
        "max_supply_label": max_supply_label,
        "next_halving_height": supply.next_halving_height,
        "blocks_until_halving": supply.blocks_until_halving,
        "emission_epoch": supply.emission_epoch,
        "mempool_transactions": mempool.transaction_count,
        "average_fee_atoms": chain.average_confirmed_fee_atoms,
        "average_fee": average_fee,
        "status": status_label,
        "label": status.network.id(),
        "index_ready": service.explorer_index_ready(),
        "index_source": index_source,
        "window": window,
    }))
}

fn network_hashrate_value(service: &NodeService) -> Result<Value, ApiError> {
    let chain = service.chain_stats();
    let network = service.network();
    Ok(json!({
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "height": chain.tip_height,
        "window_blocks": chain.hashrate_window_blocks,
        "estimated_hashrate_hps": chain.estimated_hashrate_hps,
        "estimated_hashrate": format_hashrate(chain.estimated_hashrate_hps),
    }))
}

fn network_uptime_value(service: &NodeService) -> Result<Value, ApiError> {
    let chain = service.chain_stats();
    let uptime_seconds = unix_timestamp().saturating_sub(chain.genesis_timestamp);
    let network = service.network();
    Ok(json!({
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "genesis_timestamp": chain.genesis_timestamp,
        "uptime_seconds": uptime_seconds,
        "uptime": format_uptime(uptime_seconds),
    }))
}

fn network_peers_value(service: &NodeService) -> Result<Value, ApiError> {
    let diagnostics = service.network_diagnostics();
    let network = service.network();
    let known_peer_addresses = known_peer_addresses_value(service, 128)?;
    Ok(json!({
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "active_peers": diagnostics.peer_count,
        "inbound_peers": diagnostics.inbound_peer_count,
        "outbound_peers": diagnostics.outbound_peer_count,
        "full_relay_peers": diagnostics.full_relay_peer_count,
        "block_relay_peers": diagnostics.block_relay_peer_count,
        "sync_peers": diagnostics.sync_peer_count,
        "tx_relay_peers": diagnostics.tx_relay_peer_count,
        "addr_relay_peers": diagnostics.addr_relay_peer_count,
        "topology_health_score": diagnostics.topology_health_score,
        "topology_warnings": diagnostics.topology_warnings.clone(),
        "sync_mode": diagnostics.sync_mode.clone(),
        "chain_validation_status": diagnostics.chain_validation_status.clone(),
        "safe_to_mine": diagnostics.safe_to_mine,
        "validation_lag_blocks": diagnostics.validation_lag_blocks,
        "pending_validation_blocks": diagnostics.pending_validation_blocks,
        "untrusted_downloaded_blocks": diagnostics.untrusted_downloaded_blocks,
        "connecting_peers": diagnostics.connecting_peer_count,
        "known_nodes": service.known_node_count(),
        "known_peer_addresses": known_peer_addresses,
        "bytes_sent": diagnostics.bytes_sent,
        "bytes_received": diagnostics.bytes_received,
    }))
}

fn known_peer_addresses_value(service: &NodeService, limit: usize) -> Result<Value, ApiError> {
    command_value(service, "getnodeaddresses", vec![limit.to_string()])
}

fn network_difficulty_value(service: &NodeService) -> Result<Value, ApiError> {
    let chain = service.chain_stats();
    let network = service.network();
    Ok(json!({
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "height": chain.tip_height,
        "difficulty": chain.difficulty_ratio_scaled as f64 / DIFFICULTY_DISPLAY_SCALE as f64,
        "difficulty_display": format_scaled_decimal(chain.difficulty_ratio_scaled, 8),
    }))
}

fn network_blocktime_value(service: &NodeService) -> Result<Value, ApiError> {
    let chain = service.chain_stats();
    let network = service.network();
    Ok(json!({
        "network_id": network.id(),
        "network_name": network.domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(network)),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "height": chain.tip_height,
        "window_blocks": chain.blocktime_window_blocks,
        "average_block_time_seconds": chain.average_block_time_millis as f64 / 1_000.0,
        "average_block_time": format_duration_seconds(chain.average_block_time_millis),
    }))
}

fn network_value(config: &ApiConfig, service: &NodeService) -> Result<Value, ApiError> {
    let network_info = command_value(service, "getnetworkinfo", Vec::new())?;
    let network_params = command_value(service, "getnetworkparams", Vec::new())?;
    let ruleset = command_value(service, "getrulesetinfo", Vec::new())?;
    let genesis_info = command_value(service, "getgenesisinfo", Vec::new())?;
    Ok(json!({
        "network_id": service.network().id(),
        "network_name": service.network().domain_tag(),
        "genesis_hash": hex::encode(atho_core::genesis::genesis_hash(service.network())),
        "api_version": "v1",
        "node_version": env!("CARGO_PKG_VERSION"),
        "network_info": network_info,
        "network_params": network_params,
        "ruleset": ruleset,
        "genesis": genesis_info,
        "api": {
            "version": "v1",
            "public_read_only": config.public_read_only,
            "cors_allowed_origins": config.cors.allowed_origins,
            "explorer": {
                "index_enabled": service.explorer_index_enabled(),
                "index_ready": service.explorer_index_ready(),
                "index_source": service.explorer_index_source(),
                "snapshot_enabled": service.explorer_snapshot_enabled(),
            }
        }
    }))
}

fn supply_snapshot(service: &NodeService, network: Network) -> SupplySnapshot {
    let height = service.node_ref().height();
    let params = consensus_params_for_network(network);
    let total_mined_supply_atoms =
        subsidy::cumulative_issued_through_height_for_network(network, height);
    let burned_supply_atoms = 0u128;
    let circulating_supply_atoms = service.chain_stats().circulating_supply_atoms;
    let max_supply_atoms = subsidy::max_supply_atoms_for_network(network);
    let current_block_reward_atoms =
        subsidy::block_subsidy_atoms_for_network(network, height.saturating_add(1));
    let tail_epoch =
        subsidy::TAIL_EMISSION_START_HEIGHT / subsidy::EMISSION_SCHEDULE.halving_interval_blocks;
    let emission_epoch =
        (height / subsidy::EMISSION_SCHEDULE.halving_interval_blocks).min(tail_epoch);
    let next_halving_height = if height >= subsidy::TAIL_EMISSION_START_HEIGHT {
        None
    } else {
        Some(
            emission_epoch
                .saturating_add(1)
                .saturating_mul(subsidy::EMISSION_SCHEDULE.halving_interval_blocks),
        )
    };
    let blocks_until_halving = next_halving_height.map(|next| next.saturating_sub(height));

    SupplySnapshot {
        height,
        total_mined_supply_atoms,
        circulating_supply_atoms: circulating_supply_atoms.saturating_sub(burned_supply_atoms),
        burned_supply_atoms,
        max_supply_atoms,
        current_block_reward_atoms,
        next_halving_height,
        blocks_until_halving,
        emission_epoch,
        coinbase_maturity_blocks: params.coinbase_maturity_blocks,
    }
}

fn command_value(
    service: &NodeService,
    command: &str,
    args: Vec<String>,
) -> Result<Value, ApiError> {
    let response = service.handle(RpcRequest::ExecuteCommand(CommandInvocation::new(
        command, args,
    )));
    match response {
        RpcResponse::Command(command) => Ok(command.data),
        RpcResponse::Error(error) => Err(map_rpc_error(error)),
        _ => Err(ApiError {
            status: 500,
            code: "internal_error",
        }),
    }
}

fn map_rpc_error(error: RpcError) -> ApiError {
    let lowered = error
        .details
        .as_deref()
        .unwrap_or(error.message.as_str())
        .to_ascii_lowercase();
    if lowered.contains("unknown")
        || lowered.contains("not in the mempool")
        || lowered.contains("transaction is not in the mempool")
    {
        return ApiError {
            status: 404,
            code: "not_found",
        };
    }
    ApiError {
        status: 400,
        code: "invalid_input",
    }
}

fn validate_address_for_network(address: &str, network: Network) -> Result<(), ApiError> {
    if address.len() > 128 {
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }
    let (_, decoded_network) = decode_base56_address(address).map_err(|_| ApiError {
        status: 400,
        code: "invalid_input",
    })?;
    if decoded_network != network {
        return Err(ApiError {
            status: 400,
            code: "wrong_network",
        });
    }
    Ok(())
}

fn ensure_explorer_index_available(service: &NodeService) -> Result<(), ApiError> {
    if !service.explorer_index_enabled() {
        return Err(ApiError {
            status: 503,
            code: "explorer_index_disabled",
        });
    }
    if !service.explorer_index_ready() {
        return Err(ApiError {
            status: 503,
            code: "explorer_index_not_ready",
        });
    }
    Ok(())
}

fn endpoint_requires_explorer_refresh(parts: &[&str]) -> bool {
    matches!(parts.get(2).copied(), Some("address"))
}

fn parse_hash48_value(value: &str) -> Result<[u8; 48], ApiError> {
    let bytes = hex::decode(value).map_err(|_| ApiError {
        status: 400,
        code: "invalid_input",
    })?;
    if bytes.len() != 48 {
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }
    let mut hash = [0u8; 48];
    hash.copy_from_slice(&bytes);
    Ok(hash)
}

fn classify_endpoint(path: &str) -> EndpointClass {
    if path.starts_with("/api/v1/address/")
        || path.starts_with("/api/v1/tx/")
        || path.starts_with("/api/v1/mempool/tx/")
        || path == "/api/v1/mempool"
    {
        EndpointClass::Heavy
    } else {
        EndpointClass::Standard
    }
}

fn split_url(url: &str) -> (String, BTreeMap<String, String>) {
    let mut parts = url.splitn(2, '?');
    let path = parts.next().unwrap_or("/").to_string();
    let query = parts.next().map(parse_query_string).unwrap_or_default();
    (path, query)
}

fn read_limited_body(request: &mut Request, max_bytes: usize) -> Result<String, ApiError> {
    if request
        .body_length()
        .is_some_and(|body_length| body_length > max_bytes)
    {
        return Err(ApiError {
            status: 413,
            code: "request_too_large",
        });
    }

    let mut bytes = Vec::new();
    request
        .as_reader()
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ApiError {
            status: 400,
            code: "invalid_input",
        })?;
    if bytes.len() > max_bytes {
        return Err(ApiError {
            status: 413,
            code: "request_too_large",
        });
    }
    String::from_utf8(bytes).map_err(|_| ApiError {
        status: 400,
        code: "invalid_input",
    })
}

fn parse_query_string(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or_default().to_string();
            let value = parts.next().unwrap_or_default().to_string();
            (key, value)
        })
        .collect()
}

fn parse_limit(
    query: &BTreeMap<String, String>,
    default: usize,
    max: usize,
) -> Result<usize, ApiError> {
    match query.get("limit") {
        Some(value) => {
            let parsed = value.parse::<usize>().map_err(|_| ApiError {
                status: 400,
                code: "invalid_input",
            })?;
            Ok(parsed.clamp(1, max))
        }
        None => Ok(default),
    }
}

fn parse_offset(query: &BTreeMap<String, String>) -> Result<usize, ApiError> {
    query
        .get("offset")
        .map(|value| {
            value.parse::<usize>().map_err(|_| ApiError {
                status: 400,
                code: "invalid_input",
            })
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn allow_rate_limit(bucket: &mut VecDeque<u64>, limit: usize, now: u64) -> bool {
    while bucket
        .front()
        .copied()
        .is_some_and(|seen| now.saturating_sub(seen) >= 60)
    {
        let _ = bucket.pop_front();
    }
    if bucket.len() >= limit {
        return false;
    }
    bucket.push_back(now);
    true
}

fn validate_path(path: &str) -> Result<(), ApiError> {
    if path.len() > 512 {
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }
    if path.as_bytes().contains(&0) {
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }
    Ok(())
}

fn validate_query(query: &BTreeMap<String, String>) -> Result<(), ApiError> {
    if query.len() > MAX_QUERY_PARAMS {
        return Err(ApiError {
            status: 400,
            code: "invalid_input",
        });
    }
    for (key, value) in query {
        if key.len() > MAX_QUERY_KEY_BYTES
            || value.len() > MAX_QUERY_VALUE_BYTES
            || key.as_bytes().contains(&0)
            || value.as_bytes().contains(&0)
        {
            return Err(ApiError {
                status: 400,
                code: "invalid_input",
            });
        }
    }
    Ok(())
}

fn render_block_summary(record: &atho_storage::db::BlockArchiveRecord) -> Value {
    json!({
        "height": record.height,
        "hash": hex::encode(record.block_hash),
        "previous_block_hash": hex::encode(record.previous_block_hash),
        "timestamp": record.timestamp,
        "transaction_count": record.tx_count,
        "fees_total_atoms": record.fees_total_atoms,
        "fees_total_atho": format_atoms_decimal(record.fees_total_atoms),
        "size_bytes": record.raw_block_size,
        "vsize_bytes": record.vsize_bytes,
        "weight_bytes": record.weight_bytes,
    })
}

fn render_cached_pending_transaction_summary(
    entry: &crate::service::CachedPendingTransaction,
) -> Value {
    json!({
        "txid": hex::encode(entry.txid),
        "fee_atoms": entry.fee_atoms,
        "fee": format!("{} ATHO", format_atoms_decimal(entry.fee_atoms)),
        "size_bytes": entry.size_bytes,
        "size_vbytes": entry.size_vbytes,
        "feerate_atoms_per_vbyte": entry.feerate_atoms_per_vbyte,
        "received_at": entry.received_at_unix,
    })
}

fn render_mempool_entry_value(
    network: Network,
    entry: &MempoolEntry,
    depends: &[String],
    descendants: &[String],
) -> Value {
    json!({
        "txid": hex::encode(entry.txid()),
        "wtxid": hex::encode(entry.wtxid()),
        "fee_atoms": entry.fee_atoms,
        "fee_atho": format_atoms_decimal(entry.fee_atoms),
        "base_size_bytes": entry.base_size_bytes(),
        "size_bytes": entry.raw_size_bytes(),
        "vsize_bytes": entry.vsize_bytes(),
        "feerate_atoms_per_vbyte": entry.feerate_atoms_per_vbyte(),
        "received_at_unix": entry.received_at_unix(),
        "depends": depends,
        "ancestor_count": depends.len(),
        "descendant_count": descendants.len(),
        "descendants": descendants,
        "network": network.domain_tag(),
    })
}

fn format_atoms_decimal(atoms: u64) -> String {
    let whole = atoms / ATOMS_PER_ATHO;
    let fractional = atoms % ATOMS_PER_ATHO;
    format!("{whole}.{fractional:012}")
}

fn format_u128_atoms_decimal(atoms: u128) -> String {
    let whole = atoms / ATOMS_PER_ATHO as u128;
    let fractional = atoms % ATOMS_PER_ATHO as u128;
    format!("{whole}.{fractional:012}")
}

fn format_scaled_decimal(value: u64, scale_digits: usize) -> String {
    if scale_digits == 0 {
        return value.to_string();
    }
    let divisor = 10u64.saturating_pow(scale_digits as u32).max(1);
    let whole = value / divisor;
    let fractional = value % divisor;
    format!("{whole}.{fractional:0scale_digits$}")
}

fn format_hashrate(hps: u64) -> String {
    const UNITS: [&str; 5] = ["H/s", "KH/s", "MH/s", "GH/s", "TH/s"];
    let mut value = hps as f64;
    let mut unit_index = 0usize;
    while value >= 1_000.0 && unit_index < UNITS.len().saturating_sub(1) {
        value /= 1_000.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{value:.0} {}", UNITS[unit_index])
    } else {
        format!("{value:.2} {}", UNITS[unit_index])
    }
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days} Days {hours} Hours")
    } else if hours > 0 {
        format!("{hours} Hours {minutes} Minutes")
    } else {
        format!("{minutes} Minutes")
    }
}

fn format_duration_seconds(milliseconds: u64) -> String {
    format!("{:.1}s", milliseconds as f64 / 1_000.0)
}

fn network_health_label(status: &atho_rpc::response::NodeStatus) -> &'static str {
    if !status.running {
        return "Offline";
    }
    let chain_synced = status.headers_synced && status.block_count >= status.sync_best_height;
    let peers_ok = status.network_diagnostics.peer_count > 0
        || matches!(status.network, Network::Regnet | Network::Prunetest);
    let validation_safe = status
        .network_diagnostics
        .chain_validation_status
        .is_empty()
        || status.network_diagnostics.safe_to_serve;
    if chain_synced && peers_ok && validation_safe {
        "Healthy"
    } else {
        "Warning"
    }
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn header_value(request: &Request, key: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.to_string().eq_ignore_ascii_case(key))
        .map(|header| header.value.as_str().to_string())
}

fn paginate<T>(items: &[T], limit: usize, offset: usize) -> &[T] {
    if offset >= items.len() {
        return &items[0..0];
    }
    let end = offset.saturating_add(limit).min(items.len());
    &items[offset..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NodeConfig;
    use crate::dev::{
        seed_utxo, signed_spend_transaction, signed_spend_transaction_with_signer_seed,
    };
    use crate::mempool::MempoolEntry;
    use crate::miner::Miner;
    use crate::node::Node;
    use crate::test_support::acquire_global_test_lock;
    use atho_core::address::encode_base56_address;
    use atho_core::consensus::tx_policy::minimum_required_fee_atoms;
    use atho_core::genesis;
    use atho_core::transaction::Transaction;
    use atho_crypto::falcon::generate_from_seed;
    use atho_storage::chainstate::ChainSelectionOutcome;
    use atho_storage::path::ATHO_DATA_DIR_ENV;
    use atho_storage::utxo::UtxoEntry;
    use std::ffi::OsString;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
        _lock: crate::test_support::TestLockGuard,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let lock = acquire_global_test_lock();
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn temp_data_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "atho-api-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn test_service(network: Network) -> (NodeService, EnvVarGuard) {
        let root = temp_data_dir("service");
        fs::create_dir_all(&root).expect("root");
        let guard = EnvVarGuard::set_path(ATHO_DATA_DIR_ENV, &root);
        let mut config = NodeConfig::new(network);
        if matches!(network, Network::Mainnet | Network::Testnet) {
            let keypair = generate_from_seed(b"atho-api-test-mining-reward")
                .expect("api test mining reward keypair");
            let digest = atho_core::address::public_key_digest(network, &keypair.public_key.0);
            config.mining_reward_address = encode_base56_address(network, &digest);
        }
        let mut service = NodeService::new(config);
        service.start();
        (service, guard)
    }

    fn test_state(network: Network) -> (HttpApiState, EnvVarGuard) {
        let (service, guard) = test_service(network);
        (
            HttpApiState {
                shared: Arc::new(Mutex::new(service)),
                config: NodeConfig::new(network).api,
                rate_limiter: Mutex::new(RateLimiter::default()),
            },
            guard,
        )
    }

    fn mine_with_timestamp_offset(
        node: &mut Node,
        miner: &Miner,
        offset: u64,
    ) -> atho_core::block::Block {
        let mut candidate = node.build_candidate_block().expect("candidate block");
        candidate.header.timestamp = candidate.header.timestamp.saturating_add(offset);
        candidate.header.difficulty_target_or_bits =
            node.difficulty_target_for_next_block_at(candidate.header.timestamp);
        let block = miner.solve_block(candidate);
        node.connect_block(&block).expect("connect mined block");
        block
    }

    #[test]
    fn status_and_tip_routes_render_success() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let status = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/status",
            &BTreeMap::new(),
        )
        .expect("status");
        assert_eq!(status["height"], 0);
        assert_eq!(status["api_online"], true);
        assert_eq!(status["api_version"], "v1");
        assert_eq!(status["network_id"], "atho-regnet");
        assert_eq!(status["index"]["ready"], true);
        assert_eq!(status["index"]["source"], "rebuilt");
        let tip = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/tip",
            &BTreeMap::new(),
        )
        .expect("tip");
        assert_eq!(tip["height"], 0);
    }

    #[test]
    fn address_routes_validate_network_and_return_genesis_state() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let digest = [0x44; 32];
        let address = encode_base56_address(Network::Regnet, &digest);
        service.sandbox_with_node_mut(|node| {
            node.dev_seed_chainstate(
                node.height(),
                node.tip_hash(),
                [UtxoEntry::new(
                    Network::Regnet,
                    [0x55; 48],
                    0,
                    9_999,
                    digest.to_vec(),
                    node.height(),
                    false,
                )],
            )
            .expect("seed visible utxo");
        });
        let summary = route_request(
            &config,
            &mut service,
            Network::Regnet,
            &format!("/api/v1/address/{address}"),
            &BTreeMap::new(),
        )
        .expect("summary");
        assert_eq!(summary["address"], address);
        let err = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/address/T6ADCTksAhFCSmM426CbgmfcmftE4S3yaXerTU77QzfJcJ22Wu7w",
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(err.code, "wrong_network");
    }

    #[test]
    fn invalid_txid_and_unknown_routes_fail_closed() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let bad = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/tx/not-a-hash",
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(bad.code, "invalid_input");
        let missing = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/nope",
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(missing.code, "not_found");
    }

    #[test]
    fn fees_and_network_routes_expose_read_only_policy() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;

        let fees = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/fees",
            &BTreeMap::new(),
        )
        .expect("fees");
        assert_eq!(fees["minimum_tx_fee_atoms"], MIN_TX_FEE_ATOMS);
        assert_eq!(fees["minimum_output_amount_atoms"], MIN_OUTPUT_AMOUNT_ATOMS);
        assert_eq!(fees["transaction_pow"]["hash"], "SHA3-256");

        let network = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/network",
            &BTreeMap::new(),
        )
        .expect("network");
        assert_eq!(network["api"]["public_read_only"], true);
        assert_eq!(network["network_params"]["network"], "atho-regnet");
        assert!(
            network["network_params"]["p2p_port"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
    }

    #[test]
    fn mempool_and_network_stats_routes_expose_cached_views() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let tx = Transaction {
            version: 1,
            inputs: vec![],
            outputs: vec![],
            lock_time: 0,
            witness: vec![],
            tx_pow_nonce: 7,
            tx_pow_bits: 16,
        };
        let txid = hex::encode(tx.txid());
        service.sandbox_with_node_mut(|node| {
            node.mempool.insert_unchecked(MempoolEntry::new(tx, 1_500));
        });

        let summary = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/mempool/summary",
            &BTreeMap::new(),
        )
        .expect("mempool summary");
        assert_eq!(summary["pending_transactions"], 1);
        assert_eq!(summary["status"], "Normal");
        assert_eq!(summary["recent_transactions"][0]["txid"], txid);

        let tx_view = route_request(
            &config,
            &mut service,
            Network::Regnet,
            &format!("/api/v1/mempool/tx/{txid}"),
            &BTreeMap::new(),
        )
        .expect("mempool tx");
        assert_eq!(tx_view["source"], "mempool");
        assert_eq!(tx_view["fee_atoms"], 1_500);
        assert_eq!(tx_view["fee_atho"], "0.000000001500");
        assert_eq!(tx_view["transaction"]["txid"], txid);

        let stats = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/network/stats",
            &BTreeMap::new(),
        )
        .expect("network stats");
        assert_eq!(stats["label"], "atho-regnet");
        assert_eq!(stats["network_id"], "atho-regnet");
        assert_eq!(stats["api_version"], "v1");
        assert_eq!(stats["mempool_transactions"], 1);
        assert_eq!(stats["status"], "Healthy");
        assert_eq!(stats["index_ready"], true);
        assert_eq!(stats["index_source"], "rebuilt");
        assert!(stats["circulating_supply"]
            .as_str()
            .unwrap_or_default()
            .ends_with("ATHO"));
    }

    #[test]
    fn confirmed_transaction_and_block_routes_expose_fee_fields() {
        let (mut service, _guard) = test_service(Network::Testnet);
        let config = NodeConfig::new(Network::Testnet).api;
        let miner = Miner::new(4);
        let (txid_hex, block_hash_hex, fee_atoms) = service.sandbox_with_node_mut(|node| {
            let (seed_txid, _seed_value, seed_script) = seed_utxo(Network::Testnet);
            let seed_value = 25_000u64;
            node.dev_seed_chainstate(
                6,
                node.tip_hash(),
                [UtxoEntry::new(
                    Network::Testnet,
                    seed_txid,
                    0,
                    seed_value,
                    seed_script.clone(),
                    0,
                    false,
                )],
            )
            .expect("seed chainstate");

            let tx1 = signed_spend_transaction(
                Network::Testnet,
                seed_txid,
                seed_value,
                seed_script.clone(),
            )
            .expect("signed first spend");
            let tx1id = tx1.txid();
            let tx1_output_value = tx1.outputs[0].value_atoms;
            let tx1_fee_atoms = minimum_required_fee_atoms(Network::Testnet, &tx1);
            node.admit_transaction(MempoolEntry::new(tx1, tx1_fee_atoms))
                .expect("admit first tx");
            node.mine_and_connect_candidate_block(&miner)
                .expect("mine first block");
            for _ in 0..5 {
                node.mine_and_connect_candidate_block(&miner)
                    .expect("mine confirmation block");
            }

            let tx2 = signed_spend_transaction_with_signer_seed(
                Network::Testnet,
                tx1id,
                tx1_output_value,
                seed_script.clone(),
                seed_txid,
            )
            .expect("signed second spend");
            let fee_atoms = minimum_required_fee_atoms(Network::Testnet, &tx2);
            let txid = tx2.txid();
            node.admit_transaction(MempoolEntry::new(tx2, fee_atoms))
                .expect("admit second tx");
            let block = node
                .mine_and_connect_candidate_block(&miner)
                .expect("mine second block");
            (
                hex::encode(txid),
                hex::encode(block.header.block_hash()),
                fee_atoms,
            )
        });

        let tx_view = route_request(
            &config,
            &mut service,
            Network::Testnet,
            &format!("/api/v1/tx/{txid_hex}"),
            &BTreeMap::new(),
        )
        .expect("confirmed tx");
        assert_eq!(tx_view["source"], "chain");
        assert_eq!(tx_view["fee_atoms"], fee_atoms);
        assert_eq!(tx_view["fee_atho"], format_atoms_decimal(fee_atoms));

        let block_view = route_request(
            &config,
            &mut service,
            Network::Testnet,
            &format!("/api/v1/block/hash/{block_hash_hex}"),
            &BTreeMap::new(),
        )
        .expect("block");
        assert_eq!(block_view["fees_total_atoms"], fee_atoms);
        assert_eq!(
            block_view["fees_total_atho"],
            format_atoms_decimal(fee_atoms)
        );
        assert_eq!(block_view["transactions"][1]["fee_atoms"], fee_atoms);
        assert_eq!(
            block_view["transactions"][1]["fee_atho"],
            format_atoms_decimal(fee_atoms)
        );
    }

    #[test]
    fn block_lookup_and_confirmed_transaction_routes_follow_tip() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let miner = Miner::new(1);
        let expected_tip_hash = service.sandbox_with_node_mut(|node| {
            node.mine_and_connect_candidate_block(&miner)
                .expect("mine block 1");
            let block = node
                .mine_and_connect_candidate_block(&miner)
                .expect("mine block 2");
            hex::encode(block.header.block_hash())
        });
        let expected_tip_height = service.node_ref().height();

        let latest = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/blocks/latest",
            &BTreeMap::from([(String::from("limit"), String::from("1"))]),
        )
        .expect("latest blocks");
        assert_eq!(latest["count"], 1);
        assert_eq!(latest["blocks"][0]["height"], expected_tip_height);
        assert_eq!(latest["blocks"][0]["hash"], expected_tip_hash);

        let by_height = route_request(
            &config,
            &mut service,
            Network::Regnet,
            &format!("/api/v1/block/height/{expected_tip_height}"),
            &BTreeMap::new(),
        )
        .expect("block by height");
        assert_eq!(by_height["hash"], expected_tip_hash);

        let by_hash = route_request(
            &config,
            &mut service,
            Network::Regnet,
            &format!("/api/v1/block/hash/{expected_tip_hash}"),
            &BTreeMap::new(),
        )
        .expect("block by hash");
        assert_eq!(by_hash["header"]["height"], expected_tip_height);
        assert_eq!(by_hash["hash"], expected_tip_hash);

        let genesis_state = genesis::genesis_state(Network::Regnet);
        let genesis_block = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/block/height/0",
            &BTreeMap::new(),
        )
        .expect("genesis block");
        assert_eq!(genesis_block["hash"], hex::encode(genesis_state.block_hash));

        let confirmed_tx = route_request(
            &config,
            &mut service,
            Network::Regnet,
            &format!("/api/v1/tx/{}", hex::encode(genesis_state.coinbase_txid)),
            &BTreeMap::new(),
        )
        .expect("confirmed tx");
        assert_eq!(confirmed_tx["source"], "chain");
        assert_eq!(confirmed_tx["height"], 0);
        assert_eq!(
            confirmed_tx["transaction"]["txid"],
            hex::encode(genesis_state.coinbase_txid)
        );
    }

    #[test]
    fn malformed_inputs_and_hidden_routes_fail_closed() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        for (path, expected_code) in [
            ("/api/v1/tx/not-a-hash", "invalid_input"),
            ("/api/v1/tx/", "not_found"),
            ("/api/v1/block/height/-1", "invalid_input"),
            (
                "/api/v1/block/height/999999999999999999999999999999",
                "invalid_input",
            ),
            ("/api/v1/block/hash/deadbeef", "invalid_input"),
            ("/api/v1/address/%00", "invalid_input"),
            ("/api/v1/address/\u{2603}", "invalid_input"),
            ("/api/v1/seed", "not_found"),
            ("/api/v1/private-keys", "not_found"),
            ("/api/v1/../wallet/seed", "not_found"),
        ] {
            let err = route_request(
                &config,
                &mut service,
                Network::Regnet,
                path,
                &BTreeMap::new(),
            )
            .unwrap_err();
            assert_eq!(err.code, expected_code, "path={path}");
        }

        let too_long_address = format!("/api/v1/address/{}", "A".repeat(129));
        let err = route_request(
            &config,
            &mut service,
            Network::Regnet,
            &too_long_address,
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_input");
    }

    #[test]
    fn hidden_write_and_wallet_routes_are_not_publicly_reachable() {
        let (state, _guard) = test_state(Network::Regnet);
        for path in [
            "/api/v1/miner/start",
            "/api/v1/miner/stop",
            "/api/v1/node/shutdown",
            "/api/v1/reindex",
            "/api/v1/config",
            "/api/v1/database/write",
            "/api/v1/admin/ping",
            "/api/v1/wallet/list",
        ] {
            let err = state
                .dispatch(
                    &Method::Post,
                    path,
                    &BTreeMap::new(),
                    "127.0.0.1",
                    true,
                    Some(String::from("https://atho.io")),
                )
                .unwrap_err();
            assert_eq!(err.status, 405, "path={path}");
            assert_eq!(err.code, "method_not_allowed", "path={path}");
        }

        for path in [
            "/api/v1/private-keys",
            "/api/v1/seed",
            "/api/v1/mnemonic",
            "/api/v1/admin/status",
            "/api/v1/wallet/list",
        ] {
            let err = state
                .dispatch(
                    &Method::Get,
                    path,
                    &BTreeMap::new(),
                    "127.0.0.1",
                    true,
                    Some(String::from("https://atho.io")),
                )
                .unwrap_err();
            assert_eq!(err.status, 404, "path={path}");
            assert_eq!(err.code, "not_found", "path={path}");
        }
    }

    #[test]
    fn transaction_broadcast_route_is_disabled_by_default() {
        let (state, _guard) = test_state(Network::Regnet);
        let err = state
            .dispatch_with_body(
                &Method::Post,
                "/api/v1/tx/broadcast",
                &BTreeMap::new(),
                Some(r#"{"raw_tx_hex":"00"}"#),
                "127.0.0.1",
                true,
                Some(String::from("https://atho.io")),
            )
            .unwrap_err();
        assert_eq!(err.status, 403);
        assert_eq!(err.code, "wallet_api_disabled");
    }

    #[test]
    fn transaction_broadcast_route_accepts_signed_raw_transaction_when_enabled() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let mut config = NodeConfig::new(Network::Regnet).api;
        config.wallet_enabled = true;

        let (seed_txid, seed_value, seed_script) = seed_utxo(Network::Regnet);
        service.sandbox_with_node_mut(|node| {
            node.dev_seed_chainstate(
                6,
                node.tip_hash(),
                [UtxoEntry::new(
                    Network::Regnet,
                    seed_txid,
                    0,
                    seed_value,
                    seed_script.clone(),
                    0,
                    false,
                )],
            )
            .expect("seed spendable utxo");
        });
        let transaction =
            signed_spend_transaction(Network::Regnet, seed_txid, seed_value, seed_script)
                .expect("signed transaction");
        let txid = transaction.txid();
        let fee_atoms = minimum_required_fee_atoms(Network::Regnet, &transaction);
        let body = json!({
            "raw_tx_hex": hex::encode(transaction.full_bytes()),
        })
        .to_string();
        let state = HttpApiState {
            shared: Arc::new(Mutex::new(service)),
            config,
            rate_limiter: Mutex::new(RateLimiter::default()),
        };

        let reply = state
            .dispatch_with_body(
                &Method::Post,
                "/api/v1/tx/broadcast",
                &BTreeMap::new(),
                Some(&body),
                "127.0.0.1",
                true,
                Some(String::from("https://atho.io")),
            )
            .expect("broadcast response");

        assert_eq!(reply.status, 200);
        assert_eq!(reply.body["success"], true);
        assert_eq!(reply.body["data"]["accepted"], true);
        assert_eq!(reply.body["data"]["txid"], hex::encode(txid));
        assert_eq!(reply.body["data"]["fee_atoms"], fee_atoms);
        let guard = state.shared.lock().expect("state");
        assert!(guard.node_ref().mempool_contains(&txid));
    }

    #[test]
    fn transaction_broadcast_body_accepts_json_or_plain_hex() {
        assert_eq!(
            parse_raw_transaction_request_body(r#"{"raw_tx_hex":"0abc"}"#).unwrap(),
            "0abc"
        );
        assert_eq!(
            parse_raw_transaction_request_body("0abc\n").unwrap(),
            "0abc"
        );
        assert_eq!(
            parse_raw_transaction_request_body(r#"{"fee_atoms":500}"#)
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }

    #[test]
    fn query_validation_and_response_cap_fail_safely() {
        let (service, _guard) = test_service(Network::Regnet);
        let mut config = NodeConfig::new(Network::Regnet).api;
        let state = HttpApiState {
            shared: Arc::new(Mutex::new(service)),
            config: config.clone(),
            rate_limiter: Mutex::new(RateLimiter::default()),
        };

        let err = state
            .dispatch(
                &Method::Get,
                "/api/v1/mempool",
                &BTreeMap::from([(String::from("limit"), "9".repeat(300))]),
                "127.0.0.1",
                true,
                Some(String::from("https://atho.io")),
            )
            .unwrap_err();
        assert_eq!(err.code, "invalid_input");

        let duplicate = state
            .dispatch(
                &Method::Get,
                "/api/v1/mempool",
                &parse_query_string("limit=1&limit=999&offset=0"),
                "127.0.0.1",
                true,
                Some(String::from("https://atho.io")),
            )
            .expect("duplicate params stay safe");
        assert_eq!(duplicate.status, 200);
        assert_eq!(duplicate.body["network_id"], "atho-regnet");
        assert_eq!(duplicate.body["network_name"], "regnet");
        assert_eq!(duplicate.body["api_version"], "v1");
        assert_eq!(duplicate.body["node_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(duplicate.body["data"]["page"]["limit"], 100);

        config.max_response_bytes = 128;
        let state = HttpApiState {
            shared: state.shared.clone(),
            config,
            rate_limiter: Mutex::new(RateLimiter::default()),
        };
        let err = state
            .dispatch(
                &Method::Get,
                "/api/v1/network",
                &BTreeMap::new(),
                "127.0.0.1",
                true,
                Some(String::from("https://atho.io")),
            )
            .unwrap_err();
        assert_eq!(err.status, 413);
        assert_eq!(err.code, "response_too_large");
    }

    #[test]
    fn network_stats_match_emission_logic_at_genesis() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let stats = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/network/stats",
            &BTreeMap::new(),
        )
        .expect("network stats");
        assert_eq!(stats["height"], 0);
        assert_eq!(stats["total_blocks"], 1);
        assert_eq!(
            stats["block_reward_atoms"],
            subsidy::block_subsidy_atoms_for_network(Network::Regnet, 1)
        );
        assert_eq!(
            stats["total_mined_supply_atoms"],
            subsidy::cumulative_issued_through_height_for_network(Network::Regnet, 0).to_string()
        );
        assert_eq!(stats["circulating_supply_atoms"], "0");
        assert_eq!(stats["label"], "atho-regnet");
        assert_eq!(stats["max_supply_atoms"], Value::Null);
        assert_eq!(stats["max_supply"], Value::Null);
        assert_eq!(stats["max_supply_label"], "No Fixed Cap");
        assert_eq!(stats["emission_epoch"], 0);
        assert_eq!(stats["next_halving_height"], 1_260_000);
        assert_eq!(stats["blocks_until_halving"], 1_260_000);
        assert_eq!(
            stats["latest_block_hash"],
            hex::encode(genesis::genesis_hash(Network::Regnet))
        );
    }

    #[test]
    fn supply_route_exposes_circulating_halving_and_network_fields() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let supply = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/supply",
            &BTreeMap::new(),
        )
        .expect("supply");
        assert_eq!(supply["network_id"], "atho-regnet");
        assert_eq!(
            supply["genesis_hash"],
            hex::encode(genesis::genesis_hash(Network::Regnet))
        );
        assert_eq!(
            supply["total_mined_supply_atoms"],
            subsidy::cumulative_issued_through_height_for_network(Network::Regnet, 0).to_string()
        );
        assert_eq!(supply["circulating_supply_atoms"], "0");
        assert_eq!(supply["burned_supply_atoms"], "0");
        assert_eq!(supply["max_supply_atoms"], Value::Null);
        assert_eq!(supply["max_supply"], Value::Null);
        assert_eq!(supply["max_supply_label"], "No Fixed Cap");
        assert_eq!(supply["current_block_reward_atoms"], 5_000_000_000_000u64);
        assert_eq!(supply["next_halving_height"], 1_260_000);
        assert_eq!(supply["blocks_until_halving"], 1_260_000);
        assert_eq!(supply["emission_epoch"], 0);
        assert_eq!(supply["coinbase_maturity_blocks"], 100);
    }

    #[test]
    fn reorg_refreshes_latest_block_address_index_and_stats() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let miner = Miner::new(1);
        let old_tip_hash = service.sandbox_with_node_mut(|node| {
            mine_with_timestamp_offset(node, &miner, 0);
            node.tip_hash()
        });

        let mut fork = Node::new(NodeConfig::new(Network::Regnet));
        mine_with_timestamp_offset(&mut fork, &miner, 100);
        let fork_block_2 = mine_with_timestamp_offset(&mut fork, &miner, 101);
        let fork_tip_hash = fork_block_2.header.block_hash();
        let reward_digest: [u8; 32] = fork_block_2.transactions[0].outputs[0]
            .locking_script
            .clone()
            .try_into()
            .expect("reward digest");
        let reward_address = encode_base56_address(Network::Regnet, &reward_digest);
        let fork_blocks = fork.canonical_blocks().expect("fork blocks");
        let expected_utxo_count = fork_blocks
            .iter()
            .filter(|block| {
                block.transactions[0].outputs[0].locking_script.as_slice()
                    == reward_digest.as_slice()
            })
            .count() as u64;
        let expected_balance_atoms = fork_blocks
            .iter()
            .filter_map(|block| {
                let output = &block.transactions[0].outputs[0];
                (output.locking_script.as_slice() == reward_digest.as_slice())
                    .then_some(output.value_atoms)
            })
            .sum::<u64>();

        service.sandbox_with_node_mut(|node| {
            let selection = node.consider_branch(&fork_blocks[1..]).expect("reorg");
            assert_eq!(selection.outcome, ChainSelectionOutcome::Reorged);
        });

        assert_ne!(old_tip_hash, fork_tip_hash);
        assert_eq!(service.node_ref().tip_hash(), fork_tip_hash);

        let latest = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/blocks/latest",
            &BTreeMap::from([(String::from("limit"), String::from("1"))]),
        )
        .expect("latest after reorg");
        assert_eq!(latest["blocks"][0]["hash"], hex::encode(fork_tip_hash));
        assert_eq!(latest["blocks"][0]["height"], 2);

        let stats = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/network/stats",
            &BTreeMap::new(),
        )
        .expect("stats after reorg");
        assert_eq!(stats["height"], 2);
        assert_eq!(stats["total_blocks"], 3);

        let address = route_request(
            &config,
            &mut service,
            Network::Regnet,
            &format!("/api/v1/address/{reward_address}"),
            &BTreeMap::new(),
        )
        .expect("reorg reward address");
        assert_eq!(address["utxo_count"], expected_utxo_count);
        assert_eq!(address["balance_atoms"], expected_balance_atoms);
    }

    #[test]
    fn dispatch_rejects_post_and_allows_configured_cors_preflight() {
        let (service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let state = HttpApiState {
            shared: Arc::new(Mutex::new(service)),
            config: config.clone(),
            rate_limiter: Mutex::new(RateLimiter::default()),
        };

        let post = state
            .dispatch(
                &Method::Post,
                "/api/v1/status",
                &BTreeMap::new(),
                "127.0.0.1",
                true,
                Some(String::from("https://atho.io")),
            )
            .unwrap_err();
        assert_eq!(post.status, 405);
        assert_eq!(post.code, "method_not_allowed");

        let preflight = state
            .dispatch(
                &Method::Options,
                "/api/v1/status",
                &BTreeMap::new(),
                "127.0.0.1",
                true,
                Some(String::from("https://atho.io")),
            )
            .expect("preflight");
        assert_eq!(preflight.status, 204);
        assert_eq!(preflight.allow_origin.as_deref(), Some("https://atho.io"));
        assert_eq!(preflight.body["network_id"], "atho-regnet");
        assert_eq!(preflight.body["api_version"], "v1");

        let denied = state
            .dispatch(
                &Method::Options,
                "/api/v1/status",
                &BTreeMap::new(),
                "127.0.0.1",
                true,
                None,
            )
            .unwrap_err();
        assert_eq!(denied.status, 403);
        assert_eq!(denied.code, "origin_not_allowed");
    }

    #[test]
    fn browser_requests_from_unapproved_origins_are_rejected() {
        let (service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let state = HttpApiState {
            shared: Arc::new(Mutex::new(service)),
            config,
            rate_limiter: Mutex::new(RateLimiter::default()),
        };

        let err = state
            .dispatch(
                &Method::Get,
                "/api/v1/status",
                &BTreeMap::new(),
                "127.0.0.1",
                true,
                None,
            )
            .unwrap_err();
        assert_eq!(err.status, 403);
        assert_eq!(err.code, "origin_not_allowed");
    }

    #[test]
    fn health_route_exposes_readiness_and_sync_state() {
        let (mut service, _guard) = test_service(Network::Regnet);
        let config = NodeConfig::new(Network::Regnet).api;
        let health = route_request(
            &config,
            &mut service,
            Network::Regnet,
            "/api/v1/health",
            &BTreeMap::new(),
        )
        .expect("health");
        assert_eq!(health["network_id"], "atho-regnet");
        assert_eq!(health["network_name"], "regnet");
        assert_eq!(health["api_online"], true);
        assert_eq!(health["index_ready"], true);
        assert_eq!(health["status"], "Healthy");
    }

    #[test]
    fn rate_limiter_caps_requests_per_minute() {
        let mut bucket = VecDeque::new();
        assert!(allow_rate_limit(&mut bucket, 2, 10));
        assert!(allow_rate_limit(&mut bucket, 2, 11));
        assert!(!allow_rate_limit(&mut bucket, 2, 12));
        assert!(allow_rate_limit(&mut bucket, 2, 71));
    }

    #[test]
    fn heavy_endpoint_classification_targets_the_most_expensive_public_routes() {
        assert_eq!(
            classify_endpoint("/api/v1/address/Rabc"),
            EndpointClass::Heavy
        );
        assert_eq!(
            classify_endpoint("/api/v1/tx/deadbeef"),
            EndpointClass::Heavy
        );
        assert_eq!(classify_endpoint("/api/v1/mempool"), EndpointClass::Heavy);
        assert_eq!(
            classify_endpoint("/api/v1/network/stats"),
            EndpointClass::Standard
        );
        assert_eq!(classify_endpoint("/api/v1/status"), EndpointClass::Standard);
    }

    #[test]
    fn path_validation_rejects_oversized_inputs() {
        let oversized = format!("/api/v1/tx/{}", "a".repeat(600));
        let err = validate_path(&oversized).unwrap_err();
        assert_eq!(err.code, "invalid_input");
    }
}

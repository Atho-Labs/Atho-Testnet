// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

//! Atho RPC command model, request/response types, and local transport helpers.
//!
//! This crate defines the structured RPC contract shared by the node, CLI, and
//! Qt client. It is intentionally transport-agnostic so commands can be routed
//! through local sockets or in-process calls without changing semantics.
#![forbid(unsafe_code)]

pub mod command;
pub mod error;
pub mod request;
pub mod response;
pub mod server;
pub mod transport;

//! Connection lifecycle, state machine, publish/play flows
//!
//! Mirrors `src/session/` directory.
//!
//! This module owns everything that happens after chunks have been reassembled
//! into messages:
//!
//! - [`conn`] — the per-connection [`Conn`](conn::Conn) object: buffers, chunk
//!   registry, session state, callbacks, and the publish/play authorization
//!   machinery.
//! - [`state_machine`] — the legal [`ConnState`](crate::types::ConnState)
//!   transitions.
//! - [`stream`] — per-stream bookkeeping (`publish`/`play`, pause, A/V
//!   receive flags).
//! - [`publish`] and [`play`] — the RTMP command flows that start a stream.
//! - [`publish_route`] — single-publisher-per-route enforcement.

/// Per-connection object, buffers, and authorization machinery.
pub mod conn;
/// Play command flow.
pub mod play;
/// Publish command flow.
pub mod publish;
/// Single-publisher-per-route registry.
pub mod publish_route;
/// Legal connection state transitions.
pub mod state_machine;
pub mod stream;

pub use conn::RelayFrame;

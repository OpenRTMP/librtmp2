//! Connection state machine helpers
//!
//! Mirrors `src/session/state_machine.h` and `src/session/state_machine.c`.

use crate::types::{ConnState, ErrorCode, Result};

static STATE_NAMES: &[&str] = &[
    "TCP_ACCEPTED",
    "HANDSHAKE",
    "CONNECTED",
    "CAPS_NEGOTIATED",
    "APP_CONNECTED",
    "STREAM_CREATED",
    "PUBLISHING",
    "PLAYING",
    "CLOSING",
    "CLOSED",
];

/// Attempt a state transition. Backward transitions are rejected.
///
/// `Publishing` and `Playing` are peer roles: Ord places Publishing before
/// Playing for ABI/discriminant stability, not lifecycle order, so both
/// directions between those two states are allowed.
pub fn conn_transition(current: &mut ConnState, new_state: ConnState) -> Result<()> {
    let peer_role_switch = matches!(
        (*current, new_state),
        (ConnState::Playing, ConnState::Publishing) | (ConnState::Publishing, ConnState::Playing)
    );
    if !peer_role_switch && new_state < *current {
        return Err(ErrorCode::Protocol);
    }
    *current = new_state;
    Ok(())
}

/// Get the string name of a connection state.
pub fn conn_state_str(state: ConnState) -> &'static str {
    STATE_NAMES
        .get(state as usize)
        .copied()
        .unwrap_or("UNKNOWN")
}

//! RTMP server listener
//!
//! Mirrors `src/server/server.h` and `src/server/server.c`.

use std::sync::Mutex;

use crate::session::conn::Conn;
use crate::transport::{Transport, TlsCtx};
use crate::types::*;

/// Server object.
pub struct Server {
    pub config: ServerConfig,
    pub running: bool,
    pub server_fd: i32,
    pub connections: Vec<Conn>,
    pub tls_ctx: Option<TlsCtx>,
}

impl Server {
    /// Create a new server.
    pub fn new(config: ServerConfig) -> Result<Self> {
        let tls_ctx = if config.tls_enabled != 0 {
            if config.tls_cert_file.is_null() || config.tls_key_file.is_null() {
                return Err(ErrorCode::Internal);
            }
            let cert = unsafe { std::ffi::CStr::from_ptr(config.tls_cert_file as *const i8) };
            let key = unsafe { std::ffi::CStr::from_ptr(config.tls_key_file as *const i8) };
            Some(TlsCtx::new_server(
                cert.to_str().unwrap_or(""),
                key.to_str().unwrap_or(""),
            )?)
        } else {
            None
        };

        Ok(Self {
            config,
            running: false,
            server_fd: -1,
            connections: Vec::new(),
            tls_ctx,
        })
    }

    /// Start listening on the given address.
    pub fn listen(&mut self, _bind_addr: &str) -> Result<()> {
        // In a full implementation, this would bind a TCP socket
        // For now, this is a stub that marks the server as running
        self.running = true;
        Ok(())
    }

    /// Poll for events (non-blocking).
    pub fn poll(&mut self, _timeout_ms: i32) -> Result<()> {
        if !self.running {
            return Err(ErrorCode::Internal);
        }
        // Process connections
        self.process_connections()
    }

    /// Stop the server.
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Process all active connections.
    pub fn process_connections(&mut self) -> Result<()> {
        // In a full implementation, this would recv/process/flush each connection
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.running = false;
    }
}

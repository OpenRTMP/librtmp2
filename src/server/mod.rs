//! RTMP server listener
//!
//! Mirrors `src/server/server.h` and `src/server/server.c`.

use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, IntoRawFd};

use crate::net;
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
    /// Fired for every audio/video frame on every connection.
    pub on_frame_cb: Option<fn(&Frame)>,
    listener: Option<TcpListener>,
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
            on_frame_cb: None,
            listener: None,
        })
    }

    /// Start listening on the given address ("host:port", default port 1935).
    pub fn listen(&mut self, bind_addr: &str) -> Result<()> {
        // accept_new_connections() only ever wraps incoming sockets as
        // plaintext; there is no TLS handshake wired into the accept path.
        // Refuse to start rather than silently serving TLS-configured
        // connections as plaintext.
        if self.tls_ctx.is_some() {
            return Err(ErrorCode::Unsupported);
        }

        let mut host = String::new();
        let mut port = String::new();
        net::split_host_port(bind_addr, &mut host, &mut port, "1935")?;
        let addr = if host.is_empty() {
            format!("0.0.0.0:{port}")
        } else if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };

        let listener = TcpListener::bind(&addr).map_err(|_| ErrorCode::Io)?;
        listener.set_nonblocking(true).map_err(|_| ErrorCode::Io)?;

        self.server_fd = listener.as_raw_fd();
        self.listener = Some(listener);
        self.running = true;
        Ok(())
    }

    /// Poll for events (non-blocking).
    pub fn poll(&mut self, timeout_ms: i32) -> Result<()> {
        if !self.running {
            return Err(ErrorCode::Internal);
        }
        self.accept_new_connections();
        self.process_connections()?;
        if timeout_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms as u64));
        }
        Ok(())
    }

    /// Stop the server.
    pub fn stop(&mut self) {
        self.running = false;
        self.listener = None;
    }

    /// Accept any pending inbound connections (non-blocking).
    fn accept_new_connections(&mut self) {
        let Some(listener) = self.listener.as_ref() else {
            return;
        };
        loop {
            if self.config.max_connections > 0
                && self.connections.len() >= self.config.max_connections as usize
            {
                break;
            }
            match listener.accept() {
                Ok((stream, _addr)) => {
                    let _ = stream.set_nonblocking(true);
                    let fd = stream.into_raw_fd();
                    let mut conn = Conn::new();
                    conn.client_fd = fd;
                    conn.transport = Some(Transport::new_plain(fd));
                    conn.on_frame_cb = self.on_frame_cb;
                    self.connections.push(conn);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    /// Process all active connections: drain readable bytes, drive the
    /// protocol state machine, flush pending writes, and reap closed peers.
    pub fn process_connections(&mut self) -> Result<()> {
        let mut buf = [0u8; 65536];
        let mut closed = Vec::new();

        for (i, conn) in self.connections.iter_mut().enumerate() {
            loop {
                let Some(transport) = conn.transport.as_ref() else {
                    closed.push(i);
                    break;
                };
                let mut again = 0i32;
                let n = transport.recv(&mut buf, &mut again);
                if n > 0 {
                    if conn.recv(&buf[..n as usize]).is_err() {
                        closed.push(i);
                        break;
                    }
                } else if n == 0 {
                    closed.push(i);
                    break;
                } else if again != 0 {
                    break;
                } else {
                    closed.push(i);
                    break;
                }
            }
            if conn.flush().is_err() {
                closed.push(i);
            }
        }

        for i in closed.into_iter().rev() {
            self.connections.remove(i);
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.running = false;
    }
}

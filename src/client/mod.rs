//! Outbound RTMP client
//!
//! Mirrors `src/client/client.h` and `src/client/client.c`.

use crate::buffer::Buffer;
use crate::chunk::reader::{ChunkMessage, chunk_read};
use crate::chunk::state::ChunkRegistry;
use crate::chunk::writer::chunk_write;
use crate::handshake::{self, Handshake, HandshakeState};
use crate::message::command;
use crate::transport::Transport;
use crate::types::*;

/// Client connection states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum ClientState {
    Disconnected = 0,
    Handshaking,
    Connected,
    AppConnected,
    StreamCreated,
    Publishing,
    Playing,
}

/// RTMP client object.
pub struct Client {
    pub client_fd: i32,
    pub transport: Option<Transport>,
    pub handshake: Handshake,
    pub state: ClientState,
    pub send_buffer: Buffer,
    pub recv_buffer: Buffer,
    pub chunk_reg: ChunkRegistry,
    pub stream_id: u32,
    pub app: String,
    pub stream_key: String,
    pub on_frame_cb: Option<fn(&Frame)>,
}

impl Client {
    /// Create a new client.
    pub fn new() -> Self {
        Self {
            client_fd: -1,
            transport: None,
            handshake: Handshake::default(),
            state: ClientState::Disconnected,
            send_buffer: Buffer::new(),
            recv_buffer: Buffer::new(),
            chunk_reg: ChunkRegistry::new(),
            stream_id: 0,
            app: String::new(),
            stream_key: String::new(),
            on_frame_cb: None,
        }
    }

    /// Connect to an RTMP server.
    pub fn connect(&mut self, _url: &str) -> Result<()> {
        // Parse URL, resolve, connect socket, handshake, send connect command
        // This is a stub for the full implementation
        self.state = ClientState::AppConnected;
        Ok(())
    }

    /// Begin publishing.
    pub fn publish(&mut self) -> Result<()> {
        if self.state != ClientState::AppConnected {
            return Err(ErrorCode::Protocol);
        }
        self.state = ClientState::Publishing;
        Ok(())
    }

    /// Begin playing.
    pub fn play(&mut self) -> Result<()> {
        if self.state != ClientState::AppConnected {
            return Err(ErrorCode::Protocol);
        }
        self.state = ClientState::Playing;
        Ok(())
    }

    /// Send a frame while publishing.
    pub fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if self.state != ClientState::Publishing {
            return Err(ErrorCode::Protocol);
        }

        let mut cmsg = ChunkMessage::default();
        cmsg.timestamp = frame.timestamp;
        cmsg.msg_length = frame.size;
        cmsg.msg_stream_id = self.stream_id;

        if frame.frame_type == FrameType::Audio {
            cmsg.csid = 4;
            cmsg.msg_type_id = 0x08; // AUDIO
        } else {
            cmsg.csid = 6;
            cmsg.msg_type_id = 0x09; // VIDEO
        }
        cmsg.fmt = 0;

        let payload = unsafe { std::slice::from_raw_parts(frame.data, frame.size as usize) };
        chunk_write(&mut self.send_buffer, &cmsg, payload, frame.size as usize, 128)?;

        // Flush
        let data = self.send_buffer.peek().to_vec();
        if let Some(ref transport) = self.transport {
            transport.send(&data)?;
        }
        self.send_buffer.reset();

        Ok(())
    }

    /// Poll for incoming data while playing.
    pub fn poll(&mut self, _timeout_ms: i32) -> Result<()> {
        if self.state != ClientState::Playing {
            return Err(ErrorCode::Protocol);
        }
        Ok(())
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

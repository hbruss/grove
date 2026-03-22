use std::io::{BufRead, BufReader, Error, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use crate::bridge::protocol::{
    BridgeCommand, BridgeRequestEnvelope, BridgeResponse, BridgeResponseEnvelope,
};
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct BridgeClient {
    socket_path: PathBuf,
    next_request_id: u64,
}

impl BridgeClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            next_request_id: 1,
        }
    }

    pub fn send_command(&mut self, command: BridgeCommand) -> Result<BridgeResponse> {
        let request_id = self.next_request_id();
        let request = BridgeRequestEnvelope {
            request_id: request_id.clone(),
            command,
        };
        let mut stream = UnixStream::connect(&self.socket_path)?;
        write_request(&mut stream, &request)?;
        read_response(stream, &request_id)
    }

    fn next_request_id(&mut self) -> String {
        let request_id = format!("req-{}", self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }
}

pub fn default_socket_path() -> PathBuf {
    let tmp_root = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let uid = unsafe { libc::geteuid() };
    tmp_root.join(format!("grove-bridge-{uid}.sock"))
}

fn write_request(stream: &mut UnixStream, request: &BridgeRequestEnvelope) -> Result<()> {
    let encoded = serde_json::to_string(request)
        .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?;
    stream.write_all(encoded.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn read_response(stream: UnixStream, expected_request_id: &str) -> Result<BridgeResponse> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line)?;
    if bytes_read == 0 {
        return Err(Error::new(
            ErrorKind::UnexpectedEof,
            "bridge closed connection before sending a response",
        )
        .into());
    }

    let response: BridgeResponseEnvelope = serde_json::from_str(&line)
        .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?;
    if response.request_id != expected_request_id {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "bridge response request_id mismatch: expected {expected_request_id}, got {}",
                response.request_id
            ),
        )
        .into());
    }

    Ok(response.response)
}

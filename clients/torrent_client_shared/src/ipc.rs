//! IPC Client Transport
//!
//! Provides a unified cross-platform client to exchange JSON messages with `bt_daemon`
//! over Unix domain sockets or Windows Named Pipes.

use crate::{IpcMessage, IpcReply};

/// A client for communicating with the BitTorrent background daemon.
pub struct IpcClient;

impl IpcClient {
    /// Sends an [`IpcMessage`] to the specified daemon socket/pipe and awaits an [`IpcReply`].
    pub fn send(socket_name: &str, msg: &IpcMessage) -> Result<IpcReply, String> {
        let serialized = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        let response_str = Self::send_raw(socket_name, &serialized)?;
        serde_json::from_str(&response_str).map_err(|e| e.to_string())
    }

    /// Sends a raw JSON string to the specified socket/pipe and awaits a single-line reply.
    pub fn send_raw(name: &str, payload: &str) -> Result<String, String> {
        #[cfg(unix)]
        {
            use std::io::{BufRead, BufReader, Write};
            use std::os::unix::net::UnixStream;
            let path = format!("/tmp/{}.sock", name);
            let mut stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
            let mut bytes = payload.as_bytes().to_vec();
            bytes.push(b'\n');
            stream.write_all(&bytes).map_err(|e| e.to_string())?;
            stream.flush().map_err(|e| e.to_string())?;
            let mut reader = BufReader::new(stream);
            let mut reply = String::new();
            reader.read_line(&mut reply).map_err(|e| e.to_string())?;
            Ok(reply)
        }
        #[cfg(windows)]
        {
            use std::fs::OpenOptions;
            use std::io::{BufRead, BufReader, Write};
            let pipe_path = format!(r"\\.\pipe\{}", name);
            let mut file = None;
            for _ in 0..10 {
                match OpenOptions::new().read(true).write(true).open(&pipe_path) {
                    Ok(f) => {
                        file = Some(f);
                        break;
                    }
                    Err(e) => {
                        if e.raw_os_error() == Some(231) {
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            continue;
                        }
                        return Err(e.to_string());
                    }
                }
            }
            let mut file = file.ok_or_else(|| "All pipe instances are busy".to_string())?;
            let mut bytes = payload.as_bytes().to_vec();
            bytes.push(b'\n');
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.flush().map_err(|e| e.to_string())?;
            let mut reader = BufReader::new(file);
            let mut reply = String::new();
            reader.read_line(&mut reply).map_err(|e| e.to_string())?;
            Ok(reply)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_client_send_nonexistent_socket() {
        let msg = IpcMessage::Status;
        // Connecting to a non-existent socket should return Err, not panic or deadlock
        let res = IpcClient::send("nonexistent_test_daemon_socket_12345", &msg);
        assert!(res.is_err());
    }
}

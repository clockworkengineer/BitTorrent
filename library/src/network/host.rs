//! Host network utilities
//!
//! Provides functions to query host networking attributes, such as retrieving
//! the host's external/local IP address.

use std::net::UdpSocket;

/// Attempts to retrieve the local machine's primary IP address by establishing
/// a dummy connection to a public DNS server (8.8.8.8).
/// Returns "127.0.0.1" if DNS routing fails or no connection can be made.
pub fn get_ip() -> String {
    let socket = UdpSocket::bind("0.0.0.0:0");
    if let Ok(socket) = socket {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(local_addr) = socket.local_addr() {
                return local_addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

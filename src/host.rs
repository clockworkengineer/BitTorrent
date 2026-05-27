use std::net::UdpSocket;

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

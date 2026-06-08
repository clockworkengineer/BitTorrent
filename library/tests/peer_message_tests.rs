use bittorrent_rs::{Peer, PeerMessage};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

#[test]
fn test_peer_message_encode_decode() {
    let request = PeerMessage::Request {
        index: 1,
        begin: 0,
        length: 16384,
    };
    let bytes = request.encode();
    assert_eq!(bytes.len(), 4 + 1 + 12);
    let decoded = PeerMessage::decode(&bytes[4..]).expect("Failed to decode request");
    assert_eq!(decoded, request);

    let bitfield_data = vec![0b1010_1010, 0b0101_0101];
    let bitfield = PeerMessage::Bitfield(&bitfield_data);
    let bytes = bitfield.encode();
    let decoded = PeerMessage::decode(&bytes[4..]).expect("Failed to decode bitfield");
    assert_eq!(decoded, bitfield);

    let keepalive = PeerMessage::KeepAlive;
    let bytes = keepalive.encode();
    assert_eq!(bytes, [0, 0, 0, 0]);
}

#[test]
fn test_peer_handshake_round_trip() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener
        .local_addr()
        .expect("Failed to get listener address");
    let info_hash = vec![1u8; 20];
    let info_hash_clone = info_hash.clone();
    let local_peer_id = *b"-RS0001-1234567890ab";
    let remote_peer_id = *b"-RS0001-0987654321xy";

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("Failed to accept connection");
        let mut buf = [0u8; 68];
        stream
            .read_exact(&mut buf)
            .expect("Failed to read handshake");
        assert_eq!(buf[0], 19);
        assert_eq!(&buf[1..20], b"BitTorrent protocol");
        assert_eq!(&buf[28..48], &info_hash_clone[..]);
        let reply = {
            let mut v = Vec::with_capacity(68);
            v.push(19);
            v.extend_from_slice(b"BitTorrent protocol");
            v.extend_from_slice(&[0u8; 8]);
            v.extend_from_slice(&info_hash_clone);
            v.extend_from_slice(&remote_peer_id);
            v
        };
        stream.write_all(&reply).expect("Failed to write handshake");
    });

    let stream = TcpStream::connect(addr).expect("Failed to connect to listener");
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut peer = Peer::new("127.0.0.1".to_string(), addr.port(), stream);
    let received_peer_id = futures::executor::block_on(async {
        peer.handshake(&info_hash, &local_peer_id).await
    }).expect("Handshake failed");
    assert_eq!(received_peer_id, remote_peer_id.to_vec());
    assert_eq!(
        peer.remote_peer_id.as_deref(),
        Some(&remote_peer_id.to_vec()[..])
    );
    handle.join().expect("Server thread panicked");
}

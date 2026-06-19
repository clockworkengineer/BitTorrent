//! Unit Tests for PeerNetwork Socket Wrapper
//!
//! Verifies handshake encoding/decoding, keepalive and control message parsing,
//! protocol boundary size constraints validation, and Message Stream Encryption (MSE) stream cipher obfuscation.

use bittorrent_rs::peer_network::PeerNetwork;
use bittorrent_rs::peer_message::PeerMessage;
use bittorrent_rs::MockSocket;
use std::sync::Arc;

/// Verifies that peer handshake packets are successfully encoded, transmitted, and decoded.
#[test]
fn test_peer_network_handshake_roundtrip() {
    let (socket, in_tx, out_rx) = MockSocket::new();
    let network = PeerNetwork::new(Arc::new(bittorrent_rs::Socket::Mock(socket)));

    let info_hash = vec![0x12; 20];
    let peer_id = vec![0x34; 20];

    // Seed mock socket with an incoming handshake
    let handshake_data = PeerMessage::handshake_encode(&info_hash, &peer_id).unwrap();
    in_tx.send(handshake_data).unwrap();

    futures::executor::block_on(async {
        // Test write_handshake
        let written = network.write_handshake(&info_hash, &peer_id).await.unwrap();
        assert_eq!(written, 68);

        // Test read_handshake
        let (rx_info_hash, rx_peer_id, reserved) = network.read_handshake().await.unwrap();
        assert_eq!(rx_info_hash, info_hash);
        assert_eq!(rx_peer_id, peer_id);
        assert_eq!(reserved, [0, 0, 0, 0, 0, 16, 0, 4]);
    });

    let sent = out_rx.recv().unwrap();
    assert_eq!(sent.len(), 68);
}

/// Verifies that KeepAlive, Choke, and Interested control messages are parsed correctly.
#[test]
fn test_peer_network_control_messages() {
    let (socket, in_tx, _out_rx) = MockSocket::new();
    let network = PeerNetwork::new(Arc::new(bittorrent_rs::Socket::Mock(socket)));

    // KeepAlive msg (length prefix 0)
    in_tx.send(vec![0, 0, 0, 0]).unwrap();
    
    // Choke msg (length 1, ID 0)
    in_tx.send(vec![0, 0, 0, 1, 0]).unwrap();

    // Interested msg (length 1, ID 2)
    in_tx.send(vec![0, 0, 0, 1, 2]).unwrap();

    futures::executor::block_on(async {
        let mut read_buf = vec![0u8; 64];

        let msg1 = network.read_message(&mut read_buf).await.unwrap();
        assert_eq!(msg1, PeerMessage::KeepAlive);

        let msg2 = network.read_message(&mut read_buf).await.unwrap();
        assert_eq!(msg2, PeerMessage::Choke);

        let msg3 = network.read_message(&mut read_buf).await.unwrap();
        assert_eq!(msg3, PeerMessage::Interested);
    });
}

/// Verifies that protocol constraint limits return validation errors for invalid sizes on specific message types.
#[test]
fn test_peer_network_validation_boundaries() {
    let mut read_buf = vec![0u8; 64];

    // 1. Control message Choke with invalid length 2 instead of 1 (ID 0)
    let (socket1, in_tx1, _out_rx1) = MockSocket::new();
    let network1 = PeerNetwork::new(Arc::new(bittorrent_rs::Socket::Mock(socket1)));
    in_tx1.send(vec![0, 0, 0, 2, 0, 0]).unwrap();

    futures::executor::block_on(async {
        let res = network1.read_message(&mut read_buf).await;
        assert!(res.is_err());
        assert!(format!("{:?}", res.unwrap_err()).contains("Invalid length"));
    });

    // 2. Request message with invalid length 10 instead of 13 (ID 6)
    let (socket2, in_tx2, _out_rx2) = MockSocket::new();
    let network2 = PeerNetwork::new(Arc::new(bittorrent_rs::Socket::Mock(socket2)));
    in_tx2.send(vec![0, 0, 0, 10, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();

    futures::executor::block_on(async {
        let res = network2.read_message(&mut read_buf).await;
        assert!(res.is_err());
        assert!(format!("{:?}", res.unwrap_err()).contains("Invalid length"));
    });

    // 3. Piece message with invalid length 5 (too short, must be at least 9) (ID 7)
    let (socket3, in_tx3, _out_rx3) = MockSocket::new();
    let network3 = PeerNetwork::new(Arc::new(bittorrent_rs::Socket::Mock(socket3)));
    in_tx3.send(vec![0, 0, 0, 5, 7, 0, 0, 0, 0]).unwrap();

    futures::executor::block_on(async {
        let res = network3.read_message(&mut read_buf).await;
        assert!(res.is_err());
        assert!(format!("{:?}", res.unwrap_err()).contains("Invalid length"));
    });
}

/// Verifies that Message Stream Encryption (MSE) ciphers encrypt and decrypt peer messages successfully.
#[cfg(feature = "mse")]
#[test]
fn test_peer_network_mse_integration() {
    use bittorrent_rs::mse::Rc4;

    let (socket, in_tx, out_rx) = MockSocket::new();
    let mut network = PeerNetwork::new(Arc::new(bittorrent_rs::Socket::Mock(socket)));

    let key = b"encryption_key";
    let enc = Rc4::new(key);
    let dec = Rc4::new(key);
    network.set_mse_ciphers(enc, dec);

    futures::executor::block_on(async {
        // Write standard choke message. Because MSE is set, it will be encrypted.
        network.write_message(PeerMessage::Choke).await.unwrap();
    });

    let encrypted_bytes = out_rx.recv().unwrap();
    assert_eq!(encrypted_bytes.len(), 5);
    // Ensure the message has actually been modified/encrypted (not raw [0, 0, 0, 1, 0])
    assert_ne!(encrypted_bytes, vec![0, 0, 0, 1, 0]);

    // Feed the encrypted bytes back into the incoming socket.
    // The dec cipher will decrypt it back to raw Choke.
    in_tx.send(encrypted_bytes).unwrap();

    futures::executor::block_on(async {
        let mut read_buf = vec![0u8; 64];
        let msg = network.read_message(&mut read_buf).await.unwrap();
        assert_eq!(msg, PeerMessage::Choke);
    });
}

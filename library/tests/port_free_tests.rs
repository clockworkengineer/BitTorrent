use bittorrent_rs::{AsyncSocket, BlockStorage, MemStorage, MockSocket};
use std::sync::Arc;

#[test]
fn test_mem_storage_read_write() {
    let storage = MemStorage::new(1024);
    let test_data = b"Hello BitTorrent In-Memory Storage!";
    
    // Write block
    storage.write_block(10, test_data).expect("Failed to write to MemStorage");
    
    // Read block back
    let mut read_buf = vec![0u8; test_data.len()];
    let read_len = storage.read_block(10, &mut read_buf).expect("Failed to read from MemStorage");
    
    assert_eq!(read_len, test_data.len());
    assert_eq!(&read_buf, test_data);
}

#[test]
fn test_mock_socket_communication() {
    let (socket, in_tx, out_rx) = MockSocket::new();
    let socket = Arc::new(socket);

    // Seed bytes into incoming side of MockSocket
    let test_msg = b"incoming message";
    in_tx.send(test_msg.to_vec()).expect("Failed to send incoming test bytes");

    // Run async socket read/write verification
    futures::executor::block_on(async {
        // Read incoming bytes
        let mut read_buf = vec![0u8; test_msg.len()];
        let n = socket.read(&mut read_buf).await.expect("Failed to read from MockSocket");
        assert_eq!(n, test_msg.len());
        assert_eq!(&read_buf, test_msg);

        // Write outgoing bytes
        let out_msg = b"outgoing message";
        let written = socket.write(out_msg).await.expect("Failed to write to MockSocket");
        assert_eq!(written, out_msg.len());
    });

    // Verify written bytes received on outgoing channel
    let received = out_rx.recv().expect("Failed to receive from outgoing channel");
    assert_eq!(&received, b"outgoing message");
}

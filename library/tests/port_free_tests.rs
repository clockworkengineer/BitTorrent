use bittorrent_rs::{AsyncSocket, BlockStorage, MemStorage, MockSocket, SocketFactory, BitTorrentError, Socket};
#[cfg(feature = "http-tracker")]
use bittorrent_rs::HttpClient;
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

#[derive(Debug)]
struct TestSocketFactory {
    socket: Arc<Socket>,
}

impl SocketFactory for TestSocketFactory {
    fn connect(&self, _ip: &str, _port: u16) -> Result<Arc<Socket>, BitTorrentError> {
        Ok(self.socket.clone())
    }
}

#[cfg(feature = "http-tracker")]
#[derive(Debug)]
struct TestHttpClient {
    response: Vec<u8>,
}

#[cfg(feature = "http-tracker")]
impl HttpClient for TestHttpClient {
    fn get(&self, _url: &str) -> Result<Vec<u8>, BitTorrentError> {
        Ok(self.response.clone())
    }
}

#[test]
fn test_socket_factory_injection() {
    use bittorrent_rs::SocketFactory;

    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let factory = TestSocketFactory { socket: socket.clone() };
    
    let connected_socket = factory.connect("127.0.0.1", 6881).unwrap();
    futures::executor::block_on(async {
        let written = connected_socket.write(b"hello").await.unwrap();
        assert_eq!(written, 5);
    });
}

#[cfg(feature = "http-tracker")]
#[test]
fn test_http_client_injection() {
    use bittorrent_rs::HttpClient;

    let mock_response = b"d8:intervali1800e12:min intervali1800e5:peers0:e".to_vec();
    let client = TestHttpClient { response: mock_response.clone() };
    let response = client.get("http://tracker.example.com/announce").unwrap();
    assert_eq!(response, mock_response);
}

#[test]
fn test_spinlock_mutex() {
    use bittorrent_rs::utils::io_traits::SpinLock;
    let lock = Arc::new(SpinLock::new(0));
    let lock_clone = lock.clone();

    let handle = std::thread::spawn(move || {
        let mut guard = lock_clone.lock();
        *guard += 1;
    });

    {
        let mut guard = lock.lock();
        *guard += 1;
    }

    handle.join().unwrap();
    assert_eq!(*lock.lock(), 2);
}

#[test]
fn test_mem_storage_out_of_bounds() {
    let storage = MemStorage::new(10);
    
    let err_write = storage.write_block(5, &[0; 6]);
    assert!(err_write.is_err());
    assert!(format!("{}", err_write.unwrap_err()).contains("exceeds storage capacity"));

    let mut buf = [0u8; 6];
    let err_read = storage.read_block(5, &mut buf);
    assert!(err_read.is_err());
    assert!(format!("{}", err_read.unwrap_err()).contains("exceeds storage capacity"));
}

#[test]
fn test_mock_socket_closed() {
    let (socket, _sender, _receiver) = MockSocket::new();
    socket.close();

    futures::executor::block_on(async {
        let mut buf = [0u8; 10];
        let read_res = socket.read(&mut buf).await;
        assert_eq!(read_res.unwrap(), 0);

        let write_res = socket.write(b"data").await;
        assert!(write_res.is_err());
    });
}


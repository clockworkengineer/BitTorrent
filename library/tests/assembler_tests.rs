use bittorrent_rs::Assembler;

#[test]
fn test_assembler_reservation() {
    let assembler = Assembler::new();
    
    assert!(!assembler.is_block_requested(1, 0));
    assert!(!assembler.is_block_requested(1, 1));
    
    assert!(assembler.reserve_block_request(1, 0));
    assert!(assembler.is_block_requested(1, 0));
    assert!(!assembler.is_block_requested(1, 1));
    
    assert!(!assembler.reserve_block_request(1, 0));
}

#[test]
fn test_assembler_release() {
    let assembler = Assembler::new();
    
    assembler.reserve_block_request(2, 5);
    assert!(assembler.is_block_requested(2, 5));
    
    assembler.release_block_request(2, 5);
    assert!(!assembler.is_block_requested(2, 5));
    
    assembler.release_block_request(2, 5);
}

#[test]
fn test_assembler_clear_piece() {
    let assembler = Assembler::new();
    
    assembler.reserve_block_request(3, 0);
    assembler.reserve_block_request(3, 1);
    assembler.reserve_block_request(4, 0);
    
    assert!(assembler.is_block_requested(3, 0));
    assert!(assembler.is_block_requested(3, 1));
    assert!(assembler.is_block_requested(4, 0));
    
    assembler.clear_piece_requests(3);
    
    assert!(!assembler.is_block_requested(3, 0));
    assert!(!assembler.is_block_requested(3, 1));
    assert!(assembler.is_block_requested(4, 0));
}

#[test]
fn test_piece_buffer_block_management() {
    use bittorrent_rs::piece_buffer::PieceBuffer;

    // Piece length 32768 bytes, BLOCK_SIZE is 16384, should have exactly 2 blocks.
    let mut pb = PieceBuffer::new(5, 32768);
    assert_eq!(pb.number, 5);
    assert_eq!(pb.length, 32768);
    assert_eq!(pb.blocks_present().len(), 2);
    
    // Add first block
    pb.add_block(0, "192.168.1.50");
    assert!(pb.blocks_present()[0]);
    assert!(!pb.blocks_present()[1]);
    assert_eq!(pb.block_sources[0].as_deref(), Some("192.168.1.50"));
    assert_eq!(pb.block_sources[1], None);
    assert!(!pb.all_blocks_there());

    // Duplicate add should be ignored
    pb.add_block(0, "192.168.1.99");
    assert_eq!(pb.block_sources[0].as_deref(), Some("192.168.1.50"));
}

#[test]
fn test_piece_buffer_completion() {
    use bittorrent_rs::piece_buffer::PieceBuffer;

    // Piece length 20000, should have 2 blocks (ceil(20000 / 16384))
    let mut pb = PieceBuffer::new(6, 20000);
    assert_eq!(pb.blocks_present().len(), 2);
    assert!(!pb.all_blocks_there());

    pb.add_block(0, "10.0.0.1");
    assert!(!pb.all_blocks_there());

    pb.add_block(1, "10.0.0.2");
    assert!(pb.all_blocks_there());
}

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

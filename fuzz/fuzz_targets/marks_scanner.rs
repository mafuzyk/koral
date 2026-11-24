#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut scanner = koral::marks::Scanner::default();
    let mut emitted = 0usize;
    scanner.feed(data, &mut |piece| {
        emitted += match piece {
            koral::marks::Piece::Bytes(bytes) => bytes.len(),
            koral::marks::Piece::Status(status) => status.len(),
            koral::marks::Piece::Graphics(graphics) => graphics.len(),
            koral::marks::Piece::Mark(_) => 1,
        };
    });
    assert!(
        emitted <= data.len().saturating_mul(32) + koral::marks::MAX_MARK + koral::marks::MAX_GRAPHICS
    );
});

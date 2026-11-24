#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > koral::marks::MAX_GRAPHICS + 256 {
        return;
    }
    let mut graphics = koral::graphics::Graphics::default();
    graphics.set_cell(10, 20);
    for command in data.split(|&byte| byte == b'\n').take(64) {
        graphics.handle(command);
        assert!(graphics.used() <= koral::graphics::MAX_TRANSFER);
    }
});

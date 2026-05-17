const VGA_BUFFER: *mut u8 = 0xb8000 as *mut u8;
const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

static mut CURSOR: usize = 0;

pub fn clear_screen() {
    unsafe {
        CURSOR = 0;
        for i in 0..(BUFFER_HEIGHT * BUFFER_WIDTH) {
            *VGA_BUFFER.add(i * 2) = b' ';
            *VGA_BUFFER.add(i * 2 + 1) = 0x0f; // White on black
        }
    }
}

pub fn print(s: &str) {
    unsafe {
        for &byte in s.as_bytes() {
            if byte == b'\n' {
                CURSOR = (CURSOR / BUFFER_WIDTH + 1) * BUFFER_WIDTH;
            } else {
                if CURSOR >= BUFFER_HEIGHT * BUFFER_WIDTH {
                    CURSOR = 0;
                }
                *VGA_BUFFER.add(CURSOR * 2) = byte;
                *VGA_BUFFER.add(CURSOR * 2 + 1) = 0x0a; // Classic DByteOS Light Green
                CURSOR += 1;
            }
        }
    }
}

pub fn print_byte(byte: u8) {
    unsafe {
        if byte == b'\n' {
            CURSOR = (CURSOR / BUFFER_WIDTH + 1) * BUFFER_WIDTH;
        } else {
            if CURSOR >= BUFFER_HEIGHT * BUFFER_WIDTH {
                CURSOR = 0;
            }
            *VGA_BUFFER.add(CURSOR * 2) = byte;
            *VGA_BUFFER.add(CURSOR * 2 + 1) = 0x0a; // Classic DByteOS Light Green
            CURSOR += 1;
        }
    }
}

pub fn backspace() {
    unsafe {
        if CURSOR > 0 {
            CURSOR -= 1;
            *VGA_BUFFER.add(CURSOR * 2) = b' ';
            *VGA_BUFFER.add(CURSOR * 2 + 1) = 0x0f; // White on black
        }
    }
}

#[allow(dead_code)]
pub struct VgaWriter;

impl core::fmt::Write for VgaWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        print(s);
        Ok(())
    }
}

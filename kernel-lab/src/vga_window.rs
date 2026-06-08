const VGA_BUFFER: *mut u8 = 0xB8000 as *mut u8;
const BUFFER_WIDTH: usize = 80;
const BUFFER_HEIGHT: usize = 25;
const WINDOW_ROW: usize = 2;
const WINDOW_COL: usize = 2;
const WINDOW_WIDTH: usize = 64;
const WINDOW_HEIGHT: usize = 9;
const WINDOW_ATTR: u8 = 0x0f;
const TITLE_ATTR: u8 = 0x0a;
const VALUE_ATTR: u8 = 0x0b;
const BORDER_ATTR: u8 = 0x07;

const CP437_TOP_LEFT: u8 = 0xDA;
const CP437_HORIZONTAL: u8 = 0xC4;
const CP437_TOP_RIGHT: u8 = 0xBF;
const CP437_VERTICAL: u8 = 0xB3;
const CP437_LEFT_TEE: u8 = 0xC3;
const CP437_RIGHT_TEE: u8 = 0xB4;
const CP437_BOTTOM_LEFT: u8 = 0xC0;
const CP437_BOTTOM_RIGHT: u8 = 0xD9;

pub fn draw_first_window() {
    clear_screen();
    draw_border();
    draw_text(
        WINDOW_ROW + 1,
        WINDOW_COL + 2,
        b"DBYTE.OS KERNEL LAB",
        TITLE_ATTR,
    );
    draw_status_line(3, "STATUS", "ONLINE");
    draw_status_line(4, "MODE", "VGA TEXT");
    draw_status_line(5, "IRQ0", "PREPARED  MASKED");
    draw_status_line(6, "INPUT", "PS/2 POLLING");
    draw_prompt();
}

pub fn draw_status_line(row: usize, label: &str, value: &str) {
    let y = WINDOW_ROW + row;
    draw_text(y, WINDOW_COL + 2, label.as_bytes(), WINDOW_ATTR);
    draw_text(y, WINDOW_COL + 10, value.as_bytes(), VALUE_ATTR);
}

pub fn draw_prompt() {
    let row = WINDOW_ROW + 7;
    let col = WINDOW_COL + 2;
    draw_text(row, col, b"dbyte-kernel> ", TITLE_ATTR);
    crate::vga::set_cursor(row, col + 14);
}

fn clear_screen() {
    for row in 0..BUFFER_HEIGHT {
        for col in 0..BUFFER_WIDTH {
            write_cell(row, col, b' ', WINDOW_ATTR);
        }
    }
}

fn draw_border() {
    let top = WINDOW_ROW;
    let bottom = WINDOW_ROW + WINDOW_HEIGHT - 1;
    let left = WINDOW_COL;
    let right = WINDOW_COL + WINDOW_WIDTH - 1;

    write_cell(top, left, CP437_TOP_LEFT, BORDER_ATTR);
    write_cell(top, right, CP437_TOP_RIGHT, BORDER_ATTR);
    write_cell(bottom, left, CP437_BOTTOM_LEFT, BORDER_ATTR);
    write_cell(bottom, right, CP437_BOTTOM_RIGHT, BORDER_ATTR);

    for col in (left + 1)..right {
        write_cell(top, col, CP437_HORIZONTAL, BORDER_ATTR);
        write_cell(bottom, col, CP437_HORIZONTAL, BORDER_ATTR);
    }

    for row in (top + 1)..bottom {
        write_cell(row, left, CP437_VERTICAL, BORDER_ATTR);
        write_cell(row, right, CP437_VERTICAL, BORDER_ATTR);
    }

    let separator = WINDOW_ROW + 2;
    write_cell(separator, left, CP437_LEFT_TEE, BORDER_ATTR);
    write_cell(separator, right, CP437_RIGHT_TEE, BORDER_ATTR);
    for col in (left + 1)..right {
        write_cell(separator, col, CP437_HORIZONTAL, BORDER_ATTR);
    }
}

fn draw_text(row: usize, col: usize, text: &[u8], attr: u8) {
    let max_col = WINDOW_COL + WINDOW_WIDTH - 1;
    let mut current_col = col;
    for &byte in text {
        if current_col >= max_col {
            break;
        }
        write_cell(row, current_col, byte, attr);
        current_col += 1;
    }
}

fn write_cell(row: usize, col: usize, byte: u8, attr: u8) {
    if row >= BUFFER_HEIGHT || col >= BUFFER_WIDTH {
        return;
    }
    unsafe {
        let offset = (row * BUFFER_WIDTH + col) * 2;
        *VGA_BUFFER.add(offset) = byte;
        *VGA_BUFFER.add(offset + 1) = attr;
    }
}

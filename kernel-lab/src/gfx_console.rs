use crate::vga_gfx;

const COLOR_BLACK: u8 = 0x00;
const COLOR_PANEL: u8 = 0x08;
const COLOR_BORDER: u8 = 0x07;
const COLOR_TITLE: u8 = 0x0A;
const COLOR_LABEL: u8 = 0x0F;
const COLOR_VALUE: u8 = 0x0B;
const COLOR_CURSOR: u8 = 0x0A;

const PANEL_X: usize = 16;
const PANEL_Y: usize = 10;
const PANEL_W: usize = 288;
const PANEL_H: usize = 188;
const TEXT_X: usize = PANEL_X + 16;
const VALUE_X: usize = PANEL_X + 128;
const PROMPT_TEXT: &str = "dbyte-kernel>";

pub fn draw_graphics_console() {
    vga_gfx::clear(COLOR_BLACK);
    draw_frame();
    draw_title();
    draw_status_row(PANEL_Y + 38, "KERNEL", "ONLINE");
    draw_status_row(PANEL_Y + 50, "DBYTE VM", "ONLINE");
    draw_status_row(PANEL_Y + 62, "BOOT SCRIPT", "OK");
    draw_status_row(PANEL_Y + 74, "IRQ0 TIMER", "TICKS 0008 / MASKED");
    draw_status_row(PANEL_Y + 86, "INPUT", "PS/2 POLLING");
    draw_status_row(PANEL_Y + 98, "GRAPHICS", "MODE 13H");
    draw_log_line(PANEL_Y + 116, "SYSTEM LOG");
    draw_log_line(PANEL_Y + 130, "DBYTE BOOT SCRIPT");
    draw_log_line(PANEL_Y + 142, "2");
    draw_log_line(PANEL_Y + 154, "DBYTE VM ONLINE");
    draw_log_line(PANEL_Y + 166, "42");
    draw_prompt();
}

fn draw_frame() {
    vga_gfx::fill_rect(PANEL_X, PANEL_Y, PANEL_W, PANEL_H, COLOR_PANEL);
    vga_gfx::fill_rect(PANEL_X, PANEL_Y, PANEL_W, 1, COLOR_BORDER);
    vga_gfx::fill_rect(PANEL_X, PANEL_Y + PANEL_H - 1, PANEL_W, 1, COLOR_BORDER);
    vga_gfx::fill_rect(PANEL_X, PANEL_Y, 1, PANEL_H, COLOR_BORDER);
    vga_gfx::fill_rect(PANEL_X + PANEL_W - 1, PANEL_Y, 1, PANEL_H, COLOR_BORDER);
}

fn draw_title() {
    vga_gfx::draw_text(TEXT_X, PANEL_Y + 16, "DBYTE.OS", COLOR_TITLE);
}

fn draw_status_row(y: usize, label: &str, value: &str) {
    vga_gfx::draw_text(TEXT_X, y, label, COLOR_LABEL);
    vga_gfx::draw_text(VALUE_X, y, value, COLOR_VALUE);
}

fn draw_log_line(y: usize, text: &str) {
    vga_gfx::draw_text(TEXT_X, y, text, COLOR_LABEL);
}

fn draw_prompt() {
    draw_prompt_line(PANEL_Y + 178, PROMPT_TEXT);
}

fn draw_prompt_line(y: usize, prompt: &str) {
    vga_gfx::draw_text(TEXT_X, y, prompt, COLOR_TITLE);
    let cursor_x = TEXT_X + prompt.len() * 8 + 4;
    draw_static_cursor(cursor_x, y);
}

fn draw_static_cursor(x: usize, y: usize) {
    vga_gfx::fill_rect(x, y, 6, 8, COLOR_CURSOR);
}

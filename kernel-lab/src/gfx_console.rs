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
const PROMPT_Y: usize = PANEL_Y + 178;
const PROMPT_INPUT_GAP: usize = 8;
const LOG_Y: usize = PANEL_Y + 116;
const LOG_LINE_STEP: usize = 9;
const LOG_RIGHT_X: usize = PANEL_X + PANEL_W - 16;
const LOG_ROW_W: usize = LOG_RIGHT_X - TEXT_X;
const GLYPH_W: usize = 8;

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
    clear_log_row(y);
    vga_gfx::draw_text_clipped(TEXT_X, y, text, COLOR_LABEL, LOG_RIGHT_X);
}

fn clear_log_area() {
    vga_gfx::fill_rect(TEXT_X, LOG_Y, LOG_ROW_W, PROMPT_Y - LOG_Y, COLOR_PANEL);
}

fn clear_log_row(y: usize) {
    vga_gfx::fill_rect(TEXT_X, y, LOG_ROW_W, GLYPH_W, COLOR_PANEL);
}

fn draw_log_command_line(command: &[u8]) {
    clear_log_row(LOG_Y + LOG_LINE_STEP);
    vga_gfx::draw_text_clipped(
        TEXT_X,
        LOG_Y + LOG_LINE_STEP,
        "command: ",
        COLOR_LABEL,
        LOG_RIGHT_X,
    );
    if let Ok(command_text) = core::str::from_utf8(command) {
        vga_gfx::draw_text_clipped(
            TEXT_X + 9 * GLYPH_W,
            LOG_Y + LOG_LINE_STEP,
            command_text,
            COLOR_LABEL,
            LOG_RIGHT_X,
        );
    }
}

pub fn draw_command_status_result() {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_line(LOG_Y + LOG_LINE_STEP, "command: status");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "kernel: online");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "dbyte vm: online");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 4, "boot script: ok");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 5, "irq0: ticks 0008 / masked");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 6, "input: ps/2 polling");
}

pub fn draw_command_help_result() {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_line(LOG_Y + LOG_LINE_STEP, "command: help");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "commands:");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "help status clear vm apps exit");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 4, "run <app_name>");
}

pub fn draw_command_clear_result() {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_line(LOG_Y + LOG_LINE_STEP, "command: clear");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "log: cleared");
}

pub fn draw_command_exit_result() {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_line(LOG_Y + LOG_LINE_STEP, "command: exit");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "session: closed");
}

pub fn draw_command_vm_result() {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_line(LOG_Y + LOG_LINE_STEP, "command: vm");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "DBYTE VM ONLINE");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "42");
}

pub fn draw_command_vm_error_result() {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_line(LOG_Y + LOG_LINE_STEP, "command: vm");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "result: vm error");
}

pub fn draw_command_apps_result() {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_line(LOG_Y + LOG_LINE_STEP, "command: apps");
    draw_log_line(
        LOG_Y + LOG_LINE_STEP * 2,
        "apps: hello math sysinfo ticks tickmath",
    );
}

pub fn draw_embedded_app_result(command: &[u8], output_lines: &[&str]) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);

    let mut index: usize = 0;
    while index < output_lines.len() {
        draw_log_line(LOG_Y + LOG_LINE_STEP * (index + 2), output_lines[index]);
        index += 1;
    }
}

pub fn draw_app_not_found_result(command: &[u8]) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "result: app not found");
}

pub fn draw_unknown_command_result(command: &[u8]) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "result: unknown command");
}

fn draw_prompt() {
    draw_prompt_line(PROMPT_Y, PROMPT_TEXT);
}

pub fn draw_prompt_input(input: &[u8]) {
    vga_gfx::fill_rect(TEXT_X, PROMPT_Y, PANEL_W - 32, 8, COLOR_PANEL);
    vga_gfx::draw_text(TEXT_X, PROMPT_Y, PROMPT_TEXT, COLOR_TITLE);
    let input_x = TEXT_X + PROMPT_TEXT.len() * 8 + PROMPT_INPUT_GAP;
    if let Ok(input_text) = core::str::from_utf8(input) {
        vga_gfx::draw_text(input_x, PROMPT_Y, input_text, COLOR_TITLE);
    }
    let cursor_x = input_x + input.len() * 8 + 4;
    draw_static_cursor(cursor_x, PROMPT_Y);
}

fn draw_prompt_line(y: usize, prompt: &str) {
    vga_gfx::draw_text(TEXT_X, y, prompt, COLOR_TITLE);
    let cursor_x = TEXT_X + prompt.len() * 8 + 4;
    draw_static_cursor(cursor_x, y);
}

fn draw_static_cursor(x: usize, y: usize) {
    vga_gfx::fill_rect(x, y, 6, 8, COLOR_CURSOR);
}

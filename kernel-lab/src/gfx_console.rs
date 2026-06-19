use core::fmt::Write;

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
const STATUS_IRQ0_Y: usize = PANEL_Y + 74;
const STATUS_IRQ0_VALUE_X: usize = VALUE_X - GLYPH_W * 2;
const STATUS_RIGHT_X: usize = PANEL_X + PANEL_W - 4;
const STATUS_IRQ0_VALUE_W: usize = STATUS_RIGHT_X - STATUS_IRQ0_VALUE_X;

struct FixedLineBuffer<'a> {
    bytes: &'a mut [u8],
    len: usize,
}

impl<'a> FixedLineBuffer<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, len: 0 }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl Write for FixedLineBuffer<'_> {
    fn write_str(&mut self, value: &str) -> core::fmt::Result {
        let available = self.bytes.len().saturating_sub(self.len);
        if value.len() > available {
            return Err(core::fmt::Error);
        }
        let end = self.len + value.len();
        self.bytes[self.len..end].copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub enum LastResultStatus {
    None,
    Ok,
    NotFound,
    VmError {
        name: &'static str,
        payload: Option<u8>,
    },
}

pub fn draw_graphics_console() {
    vga_gfx::clear(COLOR_BLACK);
    draw_frame();
    draw_title();
    draw_status_row(PANEL_Y + 38, "KERNEL", "ONLINE");
    draw_status_row(PANEL_Y + 50, "DBYTE VM", "ONLINE");
    draw_status_row(PANEL_Y + 62, "BOOT SCRIPT", "OK");
    draw_status_row(STATUS_IRQ0_Y, "IRQ0 TIMER", "TICKS 0008 / MASKED");
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

fn clear_irq0_status_value_row() {
    vga_gfx::fill_rect(
        STATUS_IRQ0_VALUE_X,
        STATUS_IRQ0_Y,
        STATUS_IRQ0_VALUE_W,
        GLYPH_W,
        COLOR_PANEL,
    );
}

fn draw_irq0_status_value_clipped(value: &str) {
    clear_irq0_status_value_row();
    vga_gfx::draw_text_clipped(
        STATUS_IRQ0_VALUE_X,
        STATUS_IRQ0_Y,
        value,
        COLOR_VALUE,
        STATUS_RIGHT_X,
    );
}

pub fn draw_irq0_runtime_header(
    state: &str,
    ticks: u32,
    irq0_masked: &str,
    saved_original_master_mask_valid: &str,
) {
    if saved_original_master_mask_valid != "yes" {
        draw_irq0_status_value_clipped("TICKS 0008 / MASKED");
        return;
    }

    let mut value_bytes = [0u8; 32];
    let mut value = FixedLineBuffer::new(&mut value_bytes);
    if state == "RUNNING" {
        let _ = write!(value, "RUNNING {:04}", ticks);
    } else if irq0_masked == "yes" {
        let _ = write!(value, "STOPPED {:04} / MASKED", ticks);
    } else {
        let _ = write!(value, "STOPPED {:04}", ticks);
    }
    draw_irq0_status_value_clipped(value.as_str());
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

fn draw_log_prefixed_bytes_line(y: usize, prefix: &str, value: &[u8], empty_value: &str) {
    clear_log_row(y);
    vga_gfx::draw_text_clipped(TEXT_X, y, prefix, COLOR_LABEL, LOG_RIGHT_X);
    if value.is_empty() {
        vga_gfx::draw_text_clipped(
            TEXT_X + prefix.len() * GLYPH_W,
            y,
            empty_value,
            COLOR_LABEL,
            LOG_RIGHT_X,
        );
    } else if let Ok(value_text) = core::str::from_utf8(value) {
        vga_gfx::draw_text_clipped(
            TEXT_X + prefix.len() * GLYPH_W,
            y,
            value_text,
            COLOR_LABEL,
            LOG_RIGHT_X,
        );
    } else {
        vga_gfx::draw_text_clipped(
            TEXT_X + prefix.len() * GLYPH_W,
            y,
            empty_value,
            COLOR_LABEL,
            LOG_RIGHT_X,
        );
    }
}

fn draw_log_prefixed_str_line(y: usize, prefix: &str, value: &str) {
    clear_log_row(y);
    vga_gfx::draw_text_clipped(TEXT_X, y, prefix, COLOR_LABEL, LOG_RIGHT_X);
    vga_gfx::draw_text_clipped(
        TEXT_X + prefix.len() * GLYPH_W,
        y,
        value,
        COLOR_LABEL,
        LOG_RIGHT_X,
    );
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
    draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "help status clear vm apps");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 4, "last info timer exit");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 5, "run <app_name> info <app_name>");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 6, "timer status start stop");
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
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "apps: hello math sysinfo");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "apps: ticks tickmath argtest");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 4, "apps: strtest logtest logclear");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 5, "apps: uidemo errtest");
}

pub fn draw_command_last_result(command: &[u8], app_name: &[u8], status: LastResultStatus) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);

    match status {
        LastResultStatus::None => {
            let _ = app_name;
            draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "LAST APP none");
            draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "LAST RESULT none");
        }
        LastResultStatus::Ok => {
            draw_log_prefixed_bytes_line(LOG_Y + LOG_LINE_STEP * 2, "LAST APP ", app_name, "none");
            draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "LAST RESULT APP OK");
        }
        LastResultStatus::NotFound => {
            draw_log_prefixed_bytes_line(LOG_Y + LOG_LINE_STEP * 2, "LAST APP ", app_name, "none");
            draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "LAST RESULT APP NOT FOUND");
        }
        LastResultStatus::VmError { name, payload } => {
            draw_log_prefixed_bytes_line(LOG_Y + LOG_LINE_STEP * 2, "LAST APP ", app_name, "none");
            let mut result_bytes = [0u8; 64];
            let mut result_line = FixedLineBuffer::new(&mut result_bytes);
            if let Some(value) = payload {
                draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "LAST RESULT VM ERROR");
                let _ = write!(result_line, "{}({})", name, value);
            } else {
                draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "LAST RESULT VM ERROR");
                let _ = write!(result_line, "{}", name);
            }
            draw_log_line(LOG_Y + LOG_LINE_STEP * 4, result_line.as_str());
        }
    }
}

pub fn draw_command_app_info_found(command: &[u8], app_name: &[u8], services: &str, result: &str) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);
    draw_log_prefixed_bytes_line(LOG_Y + LOG_LINE_STEP * 2, "APP ", app_name, "none");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "STATUS FOUND");
    draw_log_prefixed_str_line(LOG_Y + LOG_LINE_STEP * 4, "SERVICES ", services);
    draw_log_prefixed_str_line(LOG_Y + LOG_LINE_STEP * 5, "RESULT ", result);
}

pub fn draw_command_app_info_not_found(command: &[u8], app_name: &[u8]) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);
    draw_log_prefixed_bytes_line(LOG_Y + LOG_LINE_STEP * 2, "APP ", app_name, "none");
    draw_log_line(LOG_Y + LOG_LINE_STEP * 3, "STATUS NOT FOUND");
}

pub fn draw_command_timer_status_result(
    command: &[u8],
    state: &str,
    ticks: u32,
    irq0_masked: &str,
    sti_enabled: &str,
) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);
    draw_log_prefixed_str_line(LOG_Y + LOG_LINE_STEP * 2, "IRQ0 TIMER ", state);

    let mut tick_bytes = [0u8; 32];
    let mut tick_line = FixedLineBuffer::new(&mut tick_bytes);
    let _ = write!(tick_line, "{:04}", ticks);
    draw_log_prefixed_str_line(LOG_Y + LOG_LINE_STEP * 3, "IRQ0 TICKS ", tick_line.as_str());

    draw_log_prefixed_str_line(LOG_Y + LOG_LINE_STEP * 4, "IRQ0 MASKED ", irq0_masked);
    draw_log_prefixed_str_line(LOG_Y + LOG_LINE_STEP * 5, "STI ENABLED ", sti_enabled);
}

pub fn draw_command_timer_start_result(command: &[u8], result: &str, state: &str) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);

    if result == "irq0 runtime: not ready" {
        draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "IRQ0 RUNTIME NOT READY");
    } else if result == "irq0 runtime: already running" {
        draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "IRQ0 RUNTIME ALREADY RUNNING");
    } else if state == "RUNNING" {
        draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "IRQ0 TIMER RUNNING");
    } else {
        draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "IRQ0 RUNTIME NOT READY");
    }
}

pub fn draw_command_timer_stop_result(
    command: &[u8],
    result: &str,
    irq0_masked: &str,
    irq0_forced_masked: &str,
) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);

    if result == "irq0 runtime: not running" {
        draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "IRQ0 RUNTIME NOT RUNNING");
    } else {
        draw_log_line(LOG_Y + LOG_LINE_STEP * 2, "IRQ0 TIMER STOPPED");
        draw_log_prefixed_str_line(LOG_Y + LOG_LINE_STEP * 3, "IRQ0 MASKED ", irq0_masked);
        draw_log_prefixed_str_line(
            LOG_Y + LOG_LINE_STEP * 4,
            "IRQ0 FORCED MASKED ",
            irq0_forced_masked,
        );
    }
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

pub fn draw_embedded_app_success_result(command: &[u8], output_lines: &[&str], status: &str) {
    draw_embedded_app_result(command, output_lines);
    draw_log_line(LOG_Y + LOG_LINE_STEP * (output_lines.len() + 2), status);
}

pub fn draw_embedded_app_vm_error_result(
    command: &[u8],
    output_lines: &[&str],
    error_name: &str,
    error_payload: Option<u8>,
) {
    draw_embedded_app_result(command, output_lines);

    let mut bytes = [0u8; 48];
    let mut line = FixedLineBuffer::new(&mut bytes);
    if let Some(payload) = error_payload {
        let _ = write!(line, "VM ERROR {}({})", error_name, payload);
    } else {
        let _ = write!(line, "VM ERROR {}", error_name);
    }
    draw_log_line(
        LOG_Y + LOG_LINE_STEP * (output_lines.len() + 2),
        line.as_str(),
    );
}

pub fn draw_app_not_found_result(command: &[u8], status: &str) {
    clear_log_area();
    draw_log_line(LOG_Y, "SYSTEM LOG");
    draw_log_command_line(command);
    draw_log_line(LOG_Y + LOG_LINE_STEP * 2, status);
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

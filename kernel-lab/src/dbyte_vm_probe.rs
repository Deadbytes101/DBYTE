use core::fmt::Write;

use dbyte_kernel_vm::{Vm, VmOutput};

use crate::{serial, vga};

const DBYTE_VM_PROBE_STRINGS: [&str; 1] = ["DBYTE VM ONLINE"];
const DBYTE_VM_PROBE_BYTECODE: [u8; 17] = [
    0x02, 0x00, 0x00, // PUSH_STR_CONST 0
    0x04, // PRINT
    0x01, 0x28, 0x00, 0x00, 0x00, // PUSH_INT 40
    0x01, 0x02, 0x00, 0x00, 0x00, // PUSH_INT 2
    0x03, // ADD
    0x04, // PRINT
    0xff, // HALT
];

struct KernelVmOutput;

impl VmOutput for KernelVmOutput {
    fn write_str(&mut self, value: &str) {
        vga::print(value);
        vga::print("\n");
        serial::print(value);
        serial::print("\n");
    }

    fn write_i32(&mut self, value: i32) {
        let mut vga_writer = vga::VgaWriter;
        let mut serial_writer = serial::SerialWriter;
        let _ = writeln!(vga_writer, "{}", value);
        let _ = writeln!(serial_writer, "{}", value);
    }
}

pub fn print_status() {
    let status = "DByte kernel VM\nstate: ready\nmode: embedded bytecode\nheap: none\nfilesystem: none\n";
    vga::print(status);
    serial::print(status);
}

pub fn run_probe() {
    let mut output = KernelVmOutput;
    let mut vm = Vm::new(&DBYTE_VM_PROBE_BYTECODE, &DBYTE_VM_PROBE_STRINGS);
    if vm.run(&mut output).is_err() {
        let error = "DByte kernel VM error\n";
        vga::print(error);
        serial::print(error);
    }
}

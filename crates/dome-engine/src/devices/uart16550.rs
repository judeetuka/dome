//! A 16550 UART, which is the console.
//!
//! The status registers matter more than the data register: a guest that polls the line status
//! register and never sees transmitter-empty silently discards every character. That was
//! measured in the spike, and it is why this device reports readiness rather than zeroes.

use crate::devices::Device;

/// Where the UART lives.
pub const SERIAL_PORT: u16 = 0x3F8;
/// Length of the UART's port range.
pub const SERIAL_PORT_LENGTH: u16 = 8;
/// The memory-mapped window a guest can use instead of ports.
pub const SERIAL_MMIO_BASE: u64 = 0xc000_0000;
/// Size of the UART's window.
pub const SERIAL_MMIO_SIZE: u64 = 0x1000;

/// A UART that captures what the guest writes.
#[derive(Debug, Default)]
pub struct Uart16550 {
    output: Vec<u8>,
}

impl Uart16550 {
    /// A fresh device with nothing captured.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes the guest has written so far.
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Captured output as text, lossily.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }

    /// Forget what was captured, keeping the device attached.
    pub fn clear(&mut self) {
        self.output.clear();
    }

    /// Read a register by its port offset.
    pub fn read_port(&self, port: u16) -> u8 {
        match port {
            // Interrupt identification: no interrupt pending.
            0x02 => 0x01,
            // Line status: transmitter holding register empty, transmitter empty.
            0x05 => 0x60,
            // Modem status: CTS, DSR, DCD asserted.
            0x06 => 0xB0,
            _ => 0x00,
        }
    }

    /// Write a register by its port offset.
    pub fn write_port(&mut self, port: u16, value: u8) {
        // Offset zero is the transmit register.
        if port == 0 {
            self.output.push(value);
        }
    }

    /// Read a byte through the memory-mapped window.
    pub fn read_mmio(&self, offset: u64) -> u8 {
        self.read_port(offset as u16)
    }

    /// Write a byte through the memory-mapped window.
    pub fn write_mmio(&mut self, offset: u64, value: u8) {
        self.write_port(offset as u16, value);
    }
}

impl Device for Uart16550 {
    fn name(&self) -> &'static str {
        "uart16550"
    }

    fn mmio_range(&self) -> (u64, u64) {
        (SERIAL_MMIO_BASE, SERIAL_MMIO_SIZE)
    }

    fn mmio_read(&mut self, offset: u64, data: &mut [u8]) {
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = self.read_mmio(offset + index as u64);
        }
    }

    fn mmio_write(&mut self, offset: u64, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            self.write_mmio(offset + index as u64, *byte);
        }
    }

    fn console(&self) -> Option<&[u8]> {
        Some(&self.output)
    }

    fn io_range(&self) -> Option<(u16, u16)> {
        Some((SERIAL_PORT, SERIAL_PORT_LENGTH))
    }

    fn io_in(&mut self, port: u16, data: &mut [u8]) {
        for (index, byte) in data.iter_mut().enumerate() {
            *byte = self.read_port(port.wrapping_sub(SERIAL_PORT).wrapping_add(index as u16));
        }
    }

    fn io_out(&mut self, port: u16, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            self.write_port(
                port.wrapping_sub(SERIAL_PORT).wrapping_add(index as u16),
                *byte,
            );
        }
    }
}

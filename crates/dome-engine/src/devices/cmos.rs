//! The CMOS real-time clock.
//!
//! Every PC has one, so a guest assumes it: the kernel reads the clock to set the system time and
//! polls the status register until "update in progress" clears. A chamber without an RTC is not a
//! smaller machine, it is a machine pretending to be broken - a newer kernel polled this port
//! 40,000 times waiting for a bit that a floated bus keeps set.
//!
//! The time is deliberately fixed at the epoch. A chamber that needs the wall clock gets it from
//! the seal or from the network, not from a device whose only job here is to exist and answer.

use crate::devices::Device;

/// Index port: the register to address next.
pub const CMOS_INDEX: u16 = 0x70;
/// Data port: the register last addressed.
pub const CMOS_DATA: u16 = 0x71;

/// Register holding seconds.
const REGISTER_SECONDS: u8 = 0x00;
/// Status register A: the update-in-progress bit lives here.
const REGISTER_STATUS_A: u8 = 0x0A;
/// Status register B: time format and mode.
const REGISTER_STATUS_B: u8 = 0x0B;
/// Status register D: battery state.
const REGISTER_STATUS_D: u8 = 0x0D;

/// A CMOS clock with a fixed time.
#[derive(Debug, Default)]
pub struct Cmos {
    selected: u8,
    /// Seconds since the epoch, as a plain counter. Real hardware counts in BCD fields; the kernel
    /// reads both formats, and a counter is the one that cannot be wrong about a leap year.
    seconds: u32,
}

impl Cmos {
    /// A clock at the epoch.
    pub fn new() -> Self {
        Self::default()
    }

    /// A clock at a fixed moment, in seconds since the epoch.
    pub fn at(seconds: u32) -> Self {
        Self {
            selected: 0,
            seconds,
        }
    }

    /// Select a register, the way the index port does.
    pub fn select(&mut self, register: u8) {
        self.selected = register;
    }

    /// Read the selected register.
    pub fn read(&self) -> u8 {
        match self.selected {
            // Status A: rate divider set, and never updating. The update bit is what a driver
            // waits on, so it must be clear rather than absent.
            REGISTER_STATUS_A => 0x26,
            // Status B: 24-hour, binary rather than BCD, no daylight saving.
            REGISTER_STATUS_B => 0x02,
            // Status D: battery good.
            REGISTER_STATUS_D => 0x80,
            REGISTER_SECONDS => (self.seconds % 60) as u8,
            0x02 => ((self.seconds / 60) % 60) as u8,
            0x04 => ((self.seconds / 3600) % 24) as u8,
            0x07 => (((self.seconds / 86_400) / 365) % 100) as u8,
            0x08 => (((self.seconds / 86_400) / 365) % 100) as u8,
            0x09 => ((self.seconds / 86_400) % 100) as u8,
            _ => 0,
        }
    }
}

impl Device for Cmos {
    fn name(&self) -> &'static str {
        "cmos"
    }

    fn mmio_range(&self) -> (u64, u64) {
        // Ports only: this device has no memory window.
        (0, 0)
    }

    fn mmio_read(&mut self, _offset: u64, _data: &mut [u8]) {}

    fn mmio_write(&mut self, _offset: u64, _data: &[u8]) {}

    fn io_range(&self) -> Option<(u16, u16)> {
        Some((CMOS_INDEX, 2))
    }

    fn io_in(&mut self, port: u16, data: &mut [u8]) {
        let value = if port == CMOS_INDEX {
            self.selected
        } else {
            self.read()
        };
        data.fill(value);
    }

    fn io_out(&mut self, port: u16, data: &[u8]) {
        if port == CMOS_INDEX
            && let Some(value) = data.first()
        {
            self.select(*value);
        }
    }
}

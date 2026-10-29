//! Maestro I2C Synopsys DesignWare (AMDI0010 en portátiles AMD).

use core::ptr::{read_volatile, write_volatile};

const IC_CON: usize = 0x00;
const IC_TAR: usize = 0x04;
const IC_DATA_CMD: usize = 0x10;
const IC_SS_SCL_HCNT: usize = 0x14;
const IC_SS_SCL_LCNT: usize = 0x18;
const IC_ENABLE: usize = 0x6c;
const IC_STATUS: usize = 0x70;

const IC_STATUS_TFE: u32 = 1 << 2;
const IC_STATUS_RFNE: u32 = 1 << 3;

const CMD_READ: u32 = 1 << 8;
const CMD_STOP: u32 = 1 << 9;
const CMD_RESTART: u32 = 1 << 10;

pub struct DesignWareI2c {
    base: u64,
}

impl DesignWareI2c {
    pub fn new(mmio_phys: u64, mmio_len: u32) -> Self {
        crate::mm::ensure_mmio_mapped(mmio_phys, mmio_len as u64);
        Self { base: mmio_phys }
    }

    fn rd32(&self, off: usize) -> u32 {
        unsafe { read_volatile((self.base + off as u64) as *const u32) }
    }

    fn wr32(&self, off: usize, v: u32) {
        unsafe { write_volatile((self.base + off as u64) as *mut u32, v) }
    }

    fn wait_tx_empty(&self) {
        for _ in 0..500_000 {
            if self.rd32(IC_STATUS) & IC_STATUS_TFE != 0 {
                return;
            }
            core::hint::spin_loop();
        }
    }

    fn wait_rx(&self) -> bool {
        for _ in 0..500_000 {
            if self.rd32(IC_STATUS) & IC_STATUS_RFNE != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn enable(&self, slave: u8) {
        self.wr32(IC_ENABLE, 0);
        self.wr32(IC_CON, 0x63);
        self.wr32(IC_TAR, u32::from(slave) << 1);
        self.wr32(IC_SS_SCL_HCNT, 400);
        self.wr32(IC_SS_SCL_LCNT, 500);
        self.wr32(IC_ENABLE, 1);
        for _ in 0..1000 {
            if self.rd32(IC_ENABLE) & 1 != 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    fn disable(&self) {
        self.wr32(IC_ENABLE, 0);
    }

    /// Escribe registro de 16 bits (HID-over-I2C) y lee `len` bytes.
    pub fn read_reg16(&self, slave: u8, reg: u16, out: &mut [u8]) -> bool {
        if out.is_empty() {
            return false;
        }
        self.enable(slave);
        self.wr32(IC_DATA_CMD, (reg & 0xff) as u32);
        self.wr32(IC_DATA_CMD, ((reg >> 8) as u32) | CMD_RESTART);
        self.wait_tx_empty();
        let n = out.len();
        for (i, slot) in out.iter_mut().enumerate() {
            let stop = i + 1 == n;
            let mut cmd = CMD_READ;
            if stop {
                cmd |= CMD_STOP;
            }
            self.wr32(IC_DATA_CMD, cmd);
            if !self.wait_rx() {
                self.disable();
                return false;
            }
            *slot = (self.rd32(IC_DATA_CMD) & 0xff) as u8;
        }
        self.disable();
        true
    }
}

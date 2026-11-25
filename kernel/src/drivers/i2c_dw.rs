//! Maestro I2C Synopsys DesignWare (AMDI0010 en portátiles AMD).

use core::ptr::{read_volatile, write_volatile};

const IC_CON: usize = 0x00;
const IC_TAR: usize = 0x04;
const IC_DATA_CMD: usize = 0x10;
const IC_SS_SCL_HCNT: usize = 0x14;
const IC_SS_SCL_LCNT: usize = 0x18;
const IC_RAW_INTR_STAT: usize = 0x34;
const IC_CLR_TX_ABRT: usize = 0x54;
const IC_ENABLE: usize = 0x6c;
const IC_STATUS: usize = 0x70;
const IC_TX_ABRT_SOURCE: usize = 0x80;
const IC_ENABLE_STATUS: usize = 0x9c;
const IC_COMP_TYPE: usize = 0xfc;

/// `IC_COMP_TYPE` de un DesignWare ("DW" + 0x0140).
const DW_COMP_TYPE: u32 = 0x4457_0140;

const IC_STATUS_TFE: u32 = 1 << 2;
const IC_STATUS_RFNE: u32 = 1 << 3;
const INTR_TX_ABRT: u32 = 1 << 6;

const CMD_READ: u32 = 1 << 8;
const CMD_STOP: u32 = 1 << 9;
const CMD_RESTART: u32 = 1 << 10;

/// Vueltas de sondeo de una espera. Con un esclavo que no contesta, el
/// `TX_ABRT` corta antes.
const ESPERA: u32 = 200_000;

pub struct DesignWareI2c {
    /// Dirección **virtual** del bloque (`phys_to_virt`); `ensure_mmio_mapped`
    /// mapea ahí, no en la física. Usar la física como puntero era un page
    /// fault del kernel en cuanto se tocaba el bus.
    base: u64,
}

impl DesignWareI2c {
    /// `None` si el bloque no es un DesignWare (o está apagado y lee todo a 1).
    pub fn new(mmio_phys: u64, mmio_len: u32) -> Option<Self> {
        crate::mm::ensure_mmio_mapped(mmio_phys, mmio_len.max(0x100) as u64);
        let bus = Self {
            base: crate::mm::phys_to_virt(mmio_phys).as_u64(),
        };
        let tipo = bus.rd32(IC_COMP_TYPE);
        if tipo != DW_COMP_TYPE {
            crate::klogln!("i2c-dw: mmio={mmio_phys:#x} no es DesignWare (comp_type={tipo:#x})");
            return None;
        }
        Some(bus)
    }

    fn rd32(&self, off: usize) -> u32 {
        unsafe { read_volatile((self.base + off as u64) as *const u32) }
    }

    fn wr32(&self, off: usize, v: u32) {
        unsafe { write_volatile((self.base + off as u64) as *mut u32, v) }
    }

    /// `true` si el esclavo no respondió (NACK) o la transferencia se abortó.
    fn abortada(&self) -> bool {
        if self.rd32(IC_RAW_INTR_STAT) & INTR_TX_ABRT == 0 {
            return false;
        }
        let _ = self.rd32(IC_TX_ABRT_SOURCE);
        let _ = self.rd32(IC_CLR_TX_ABRT);
        true
    }

    fn wait_tx_empty(&self) -> bool {
        for _ in 0..ESPERA {
            if self.abortada() {
                return false;
            }
            if self.rd32(IC_STATUS) & IC_STATUS_TFE != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn wait_rx(&self) -> bool {
        for _ in 0..ESPERA {
            if self.rd32(IC_STATUS) & IC_STATUS_RFNE != 0 {
                return true;
            }
            if self.abortada() {
                return false;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn enable(&self, slave: u8) {
        self.disable();
        // Maestro, modo estándar, RESTART permitido, esclavo apagado.
        self.wr32(IC_CON, 0x63);
        // `IC_TAR` lleva la dirección de 7 bits tal cual (no desplazada).
        self.wr32(IC_TAR, u32::from(slave));
        // ~100 kHz con el reloj de 150 MHz de los AMD (más lento si es menor).
        self.wr32(IC_SS_SCL_HCNT, 600);
        self.wr32(IC_SS_SCL_LCNT, 750);
        let _ = self.rd32(IC_CLR_TX_ABRT);
        self.wr32(IC_ENABLE, 1);
        for _ in 0..1000 {
            if self.rd32(IC_ENABLE_STATUS) & 1 != 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    fn disable(&self) {
        self.wr32(IC_ENABLE, 0);
        for _ in 0..1000 {
            if self.rd32(IC_ENABLE_STATUS) & 1 == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    /// Escribe `datos` al esclavo en una sola transferencia (STOP al final).
    pub fn write(&self, slave: u8, datos: &[u8]) -> bool {
        if datos.is_empty() {
            return false;
        }
        self.enable(slave);
        for (i, &b) in datos.iter().enumerate() {
            let mut cmd = u32::from(b);
            if i + 1 == datos.len() {
                cmd |= CMD_STOP;
            }
            self.wr32(IC_DATA_CMD, cmd);
        }
        let ok = self.wait_tx_empty();
        // TFE llega antes de que salga el último bit: un NACK tardío se ve aquí.
        let ok = ok && !self.abortada();
        self.disable();
        ok
    }

    /// Escribe registro de 16 bits (HID-over-I2C) y lee `len` bytes.
    pub fn read_reg16(&self, slave: u8, reg: u16, out: &mut [u8]) -> bool {
        if out.is_empty() {
            return false;
        }
        self.enable(slave);
        self.wr32(IC_DATA_CMD, (reg & 0xff) as u32);
        self.wr32(IC_DATA_CMD, (reg >> 8) as u32);
        let n = out.len();
        for (i, slot) in out.iter_mut().enumerate() {
            // El cambio de escritura a lectura va con RESTART en la primera
            // lectura; STOP en la última.
            let mut cmd = CMD_READ;
            if i == 0 {
                cmd |= CMD_RESTART;
            }
            if i + 1 == n {
                cmd |= CMD_STOP;
            }
            self.wr32(IC_DATA_CMD, cmd);
            if !self.wait_rx() {
                self.disable();
                return false;
            }
            *slot = (self.rd32(IC_DATA_CMD) & 0xff) as u8;
        }
        let ok = self.wait_tx_empty();
        self.disable();
        ok
    }
}

//! Touchpad interno vía HID-over-I2C (PNP0C50 + AMDI0010).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;
use xhci_nostd::hid::{PointerReportLayout, PointerState};

use super::i2c_dw::DesignWareI2c;
use crate::arch::acpi_hid_i2c::{self, HidI2cBinding};

static IRQ_PENDIENTE: AtomicBool = AtomicBool::new(false);

pub fn marcar_irq_pendiente() {
    IRQ_PENDIENTE.store(true, Ordering::Release);
    crate::net::marcar_trabajo_pendiente();
}

pub fn tomar_irq_pendiente() -> bool {
    IRQ_PENDIENTE.swap(false, Ordering::AcqRel)
}

/// Hay algún touchpad (el sondeo sale sin mirar nada si no).
static HAY: AtomicBool = AtomicBool::new(false);
/// Todos los touchpads avisan por IRQ: el sondeo es sólo de respaldo.
static CON_ATENCION: AtomicBool = AtomicBool::new(true);
/// La atención va por `amd_gpio`, que puede enmascararla si la línea no baja.
static POR_GPIO: AtomicBool = AtomicBool::new(false);
static ULTIMO_SONDEO: AtomicU64 = AtomicU64::new(0);
/// Una lectura del informe son 3–6 ms de bus a 100 kHz: sondear en cada
/// pasada del planificador se comía la CPU. Sin IRQ, cada 25 ms; con IRQ,
/// la lectura la dispara la interrupción y esto es sólo de respaldo.
const SONDEO_SIN_IRQ_MS: u64 = 25;
const SONDEO_CON_IRQ_MS: u64 = 250;
/// Diagnóstico: lecturas hechas, saltadas por la línea inactiva, vacías
/// (longitud ≤ 2) y con datos. Se vuelcan cuando cambian, como mucho cada
/// [`DIAG_MS`] y [`DIAG_MAX`] veces.
static LECTURAS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SALTADAS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static VACIAS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static CON_DATOS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static DIAG_ULTIMO: AtomicU64 = AtomicU64::new(0);
static DIAG_FIRMA: AtomicU64 = AtomicU64::new(0);
static DIAG_VECES: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
const DIAG_MS: u64 = 2000;
const DIAG_MAX: u32 = 40;

fn quizas_diag(ahora: u64) {
    if ahora.saturating_sub(DIAG_ULTIMO.load(Ordering::Relaxed)) < DIAG_MS
        || DIAG_VECES.load(Ordering::Relaxed) >= DIAG_MAX
    {
        return;
    }
    let r = |c: &core::sync::atomic::AtomicU32| c.load(Ordering::Relaxed);
    let irqs = r(&super::amd_gpio::IRQS);
    let (l, s, v, d) = (r(&LECTURAS), r(&SALTADAS), r(&VACIAS), r(&CON_DATOS));
    let nivel = super::amd_gpio::nivel();
    let firma = (irqs as u64) ^ ((l as u64) << 12) ^ ((v as u64) << 24) ^ ((d as u64) << 36)
        ^ ((nivel == Some(true)) as u64) << 63;
    DIAG_ULTIMO.store(ahora, Ordering::Relaxed);
    if DIAG_FIRMA.swap(firma, Ordering::Relaxed) == firma {
        return;
    }
    DIAG_VECES.fetch_add(1, Ordering::Relaxed);
    crate::klogln!(
        "hid-i2c: diag t={ahora} irq={irqs} lecturas={l} saltadas={s} vacias={v} con_datos={d} nivel_alto={nivel:?}"
    );
}

/// Lecturas fallidas seguidas antes de dejar de hablarle a un touchpad.
const MAX_FALLOS: u32 = 50;

struct HidI2cDevice {
    bus: DesignWareI2c,
    slave: u8,
    input_reg: u16,
    max_input: u16,
    state: PointerState,
    fallos: u32,
    /// Informes no vacíos que quedan por volcar al registro (diagnóstico).
    volcar: u8,
}

static DEVICES: Mutex<Vec<HidI2cDevice>> = Mutex::new(Vec::new());

#[repr(C, packed)]
struct HidDesc {
    len: u16,
    bcd: u16,
    rdesc_len: u16,
    rdesc_reg: u16,
    input_reg: u16,
    max_input: u16,
    output_reg: u16,
    max_output: u16,
    command_reg: u16,
}

/// Opcode HID-over-I2C `SET_POWER` (estado 0 = encendido).
const OP_SET_POWER: u8 = 0x08;
/// Opcode HID-over-I2C `RESET`.
const OP_RESET: u8 = 0x01;

pub fn init() {
    let bindings = acpi_hid_i2c::discover();
    let mut devs = DEVICES.lock();
    for b in bindings.iter().copied() {
        if devs.iter().any(|d| d.slave == b.slave) {
            continue;
        }
        if let Some(d) = probe_binding(b) {
            crate::println!("hid-i2c: esclavo {:#04x} responde en mmio={:#x}", b.slave, b.i2c_mmio);
            if !arm_attention(b) {
                CON_ATENCION.store(false, Ordering::Relaxed);
            }
            devs.push(d);
        }
    }
    if !devs.is_empty() {
        HAY.store(true, Ordering::Release);
        crate::println!("hid-i2c: {} touchpad(s) listo(s)", devs.len());
    }
}

fn hid_gsi_irq() {
    marcar_irq_pendiente();
}

/// `true` si el touchpad quedó avisando por interrupción.
fn arm_attention(b: HidI2cBinding) -> bool {
    if let Some(gsi) = b.gsi {
        if let Some(vec) = crate::arch::irq::allocate(hid_gsi_irq) {
            match crate::arch::ioapic::route(gsi, vec, crate::arch::apic::id(), b.gsi_flags) {
                Ok(()) => {
                    crate::println!("hid-i2c: irq gsi={gsi} vector={vec:#x}");
                    return true;
                }
                Err(e) => crate::klogln!("hid-i2c: route gsi={gsi} fallo ({e})"),
            }
        }
    }
    let Some(pin) = b.gpio_pin else {
        crate::klogln!("hid-i2c: sin GSI ni GpioInt; solo sondeo");
        return false;
    };
    let Some(gpio) = acpi_hid_i2c::discover_amd_gpio() else {
        crate::klogln!("hid-i2c: GpioInt pin={pin} sin AMDI0030");
        return false;
    };
    let Some(gsi) = gpio.gsi else {
        crate::klogln!("hid-i2c: AMDI0030 sin Interrupt");
        return false;
    };
    match super::amd_gpio::arm_pin(gpio.mmio, gpio.len, pin, gsi, gpio.gsi_flags, b.gpio_activo_alto) {
        Ok(()) => {
            POR_GPIO.store(true, Ordering::Relaxed);
            crate::println!(
                "hid-i2c: atención por amd-gpio pin={pin} gsi={gsi} activo_alto={} nivel_alto={:?} activa={:?}",
                b.gpio_activo_alto,
                super::amd_gpio::nivel(),
                super::amd_gpio::atencion_activa()
            );
            true
        }
        Err(e) => {
            crate::klogln!("hid-i2c: amd-gpio pin={pin} fallo ({e})");
            false
        }
    }
}

/// Sondeo desde `poll_devices`: limitado en frecuencia.
pub fn poll() {
    sondear(false);
}

/// Lectura tras una interrupción del touchpad (camino blando de IRQ).
pub fn poll_por_irq() {
    sondear(true);
}

fn sondear(por_irq: bool) {
    if !HAY.load(Ordering::Acquire) {
        return;
    }
    let ahora = crate::arch::pit::uptime_ms();
    if !por_irq {
        let gpio_caido =
            POR_GPIO.load(Ordering::Relaxed) && super::amd_gpio::atencion_activa().is_none();
        let periodo = if CON_ATENCION.load(Ordering::Relaxed) && !gpio_caido {
            SONDEO_CON_IRQ_MS
        } else {
            SONDEO_SIN_IRQ_MS
        };
        if ahora.saturating_sub(ULTIMO_SONDEO.load(Ordering::Relaxed)) < periodo {
            return;
        }
    }
    ULTIMO_SONDEO.store(ahora, Ordering::Relaxed);
    // Nada de registro desde el camino de IRQ: `klogln!` coge un candado.
    if !por_irq {
        quizas_diag(ahora);
    }
    // Nunca esperar: lo llaman el planificador y el camino blando de IRQ.
    let Some(mut devs) = DEVICES.try_lock() else {
        return;
    };
    // Con la línea de atención por GPIO, sólo hay informe si está activa.
    // Leer con la línea inactiva (el flanco de subida al soltarla, o el
    // sondeo de respaldo) devolvía otra vez el último informe: el puntero
    // se movía solo.
    if super::amd_gpio::atencion_activa() == Some(false) {
        SALTADAS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    for dev in devs.iter_mut() {
        if dev.fallos >= MAX_FALLOS {
            continue;
        }
        let mut buf = [0u8; 64];
        let len = dev.max_input.min(64) as usize;
        if !dev
            .bus
            .read_reg16(dev.slave, dev.input_reg, &mut buf[..len])
        {
            dev.fallos += 1;
            if dev.fallos == MAX_FALLOS && !por_irq {
                crate::klogln!(
                    "hid-i2c: esclavo {:#04x}: {MAX_FALLOS} lecturas fallidas seguidas; lo dejo",
                    dev.slave
                );
            }
            continue;
        }
        dev.fallos = 0;
        // El informe de HID-over-I2C empieza con su longitud (2 bytes, que se
        // cuentan). 0 o 2 = no hay nada; lo que sigue es Report ID + datos.
        LECTURAS.fetch_add(1, Ordering::Relaxed);
        let total = u16::from_le_bytes([buf[0], buf[1]]) as usize;
        if total <= 2 {
            VACIAS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        CON_DATOS.fetch_add(1, Ordering::Relaxed);
        let informe = &buf[2..total.min(len)];
        if dev.volcar > 0 && !por_irq {
            dev.volcar -= 1;
            crate::klogln!("hid-i2c: informe {:02x?}", informe);
        }
        dev.state.process_raw_report(informe);
        while let Some(evt) = dev.state.next_event() {
            super::mouse::apply_hid_pointer(evt);
        }
    }
}

/// Registros donde los fabricantes suelen poner el descriptor HID. El de
/// verdad lo da el `_DSM` del dispositivo en ACPI, que no ejecutamos: se
/// prueban estos y vale el primero con un descriptor bien formado.
const REGISTROS_DESCRIPTOR: [u16; 5] = [0x0001, 0x0020, 0x0000, 0x0030, 0x0010];

/// (registro, longitud) del descriptor HID: `wHIDDescLength` = 30 y
/// `bcdVersion` = 1.00, como fija la especificación HID-over-I2C.
fn buscar_descriptor(bus: &DesignWareI2c, b: HidI2cBinding) -> Option<(u16, usize)> {
    let mut respondio = false;
    for reg in REGISTROS_DESCRIPTOR {
        let mut hdr = [0u8; 4];
        if !bus.read_reg16(b.slave, reg, &mut hdr) {
            continue;
        }
        respondio = true;
        let len = u16::from_le_bytes([hdr[0], hdr[1]]);
        let bcd = u16::from_le_bytes([hdr[2], hdr[3]]);
        if len == 30 && bcd == 0x0100 {
            crate::println!(
                "hid-i2c: esclavo {:#04x} en mmio={:#x}: descriptor HID en el registro {reg:#06x}",
                b.slave,
                b.i2c_mmio
            );
            return Some((reg, len as usize));
        }
        crate::klogln!(
            "hid-i2c: esclavo {:#04x} en mmio={:#x} registro {reg:#06x}: {:02x?}",
            b.slave,
            b.i2c_mmio,
            hdr
        );
    }
    if !respondio {
        crate::klogln!("hid-i2c: esclavo {:#04x} no responde en mmio={:#x}", b.slave, b.i2c_mmio);
    }
    None
}

fn probe_binding(b: HidI2cBinding) -> Option<HidI2cDevice> {
    let bus = DesignWareI2c::new(b.i2c_mmio, b.i2c_len)?;
    let Some((reg, desc_len)) = buscar_descriptor(&bus, b) else {
        return None;
    };
    let mut raw = vec![0u8; desc_len];
    if !bus.read_reg16(b.slave, reg, &mut raw) {
        return None;
    }
    let hd = unsafe { &*(raw.as_ptr() as *const HidDesc) };
    // Encender: sin esto, más de un touchpad no entrega informes.
    let cmd = hd.command_reg;
    if !bus.write(b.slave, &[cmd as u8, (cmd >> 8) as u8, 0x00, OP_SET_POWER]) {
        crate::klogln!("hid-i2c: esclavo {:#04x} no aceptó SET_POWER", b.slave);
    }
    crate::arch::tsc::delay_ms(10);
    // RESET, como Linux tras encender: el dispositivo contesta con un informe
    // de longitud 0 y levanta la atención; hasta leerlo, algunos la retienen.
    if bus.write(b.slave, &[cmd as u8, (cmd >> 8) as u8, 0x00, OP_RESET]) {
        crate::arch::tsc::delay_ms(100);
        let mut r = [0u8; 4];
        let ok = bus.read_reg16(b.slave, hd.input_reg, &mut r);
        crate::klogln!("hid-i2c: esclavo {:#04x} RESET → {ok} {:02x?}", b.slave, r);
    } else {
        crate::klogln!("hid-i2c: esclavo {:#04x} no aceptó RESET", b.slave);
    }
    let rdesc_len = hd.rdesc_len as usize;
    if rdesc_len == 0 || rdesc_len > 4096 {
        return None;
    }
    let mut rdesc = vec![0u8; rdesc_len];
    if !bus.read_reg16(b.slave, hd.rdesc_reg, &mut rdesc) {
        crate::klogln!("hid-i2c: report descriptor ilegible");
        return None;
    }
    // El descriptor al registro, para poder analizarlo fuera de la placa.
    for (n, trozo) in rdesc.chunks(32).enumerate() {
        crate::klogln!("hid-i2c: rdesc {:04x}: {:02x?}", n * 32, trozo);
    }
    let layout = PointerReportLayout::from_descriptor(&rdesc).unwrap_or_else(|| {
        crate::klogln!("hid-i2c: descriptor sin puntero; boot fallback");
        PointerReportLayout::boot()
    });
    crate::klogln!("hid-i2c: formato {:?}", layout);
    Some(HidI2cDevice {
        bus,
        slave: b.slave,
        input_reg: hd.input_reg,
        max_input: hd.max_input.max(8),
        state: PointerState::new(layout),
        fallos: 0,
        volcar: 8,
    })
}

//! Inventario ACPI: controladores AMDI0010/AMDI0030 y touchpads PNP0C50.

use alloc::vec::Vec;

use super::acpi;

#[derive(Clone, Copy, Debug)]
pub struct HidI2cBinding {
    pub i2c_mmio: u64,
    pub i2c_len: u32,
    pub slave: u8,
    /// GSI directo del dispositivo (Interrupt extendido), si existe.
    pub gsi: Option<u32>,
    pub gsi_flags: u16,
    /// Pin GpioInt (AMDI0030), si no hay GSI directo.
    pub gpio_pin: Option<u16>,
}

#[derive(Clone, Copy, Debug)]
pub struct AmdGpioBlock {
    pub mmio: u64,
    pub len: u32,
    pub gsi: Option<u32>,
    pub gsi_flags: u16,
}

pub fn discover() -> Vec<HidI2cBinding> {
    let mut mmios: Vec<(u64, u32)> = Vec::new();
    let mut slaves: Vec<(u8, Option<u32>, u16, Option<u16>)> = Vec::new();
    acpi::each_table(b"DSDT", |aml| scan_aml(aml, &mut mmios, &mut slaves));
    acpi::each_table(b"SSDT", |aml| scan_aml(aml, &mut mmios, &mut slaves));
    if mmios.is_empty() {
        crate::klogln!("hid-i2c: sin AMDI0010 en ACPI");
        informar_candidatos();
        return Vec::new();
    }
    if slaves.is_empty() {
        crate::klogln!("hid-i2c: sin PNP0C50 en ACPI");
        informar_candidatos();
        return Vec::new();
    }
    // No se sabe de qué controlador cuelga cada touchpad (el descriptor I2C
    // lo nombra por ruta ACPI): se ofrece cada esclavo en cada controlador y
    // `hid_i2c::init` se queda con el primero que responde con su descriptor.
    let mut out = Vec::new();
    for &(slave, gsi, gsi_flags, gpio_pin) in &slaves {
        for &(i2c_mmio, i2c_len) in &mmios {
            out.push(HidI2cBinding {
                i2c_mmio,
                i2c_len,
                slave,
                gsi,
                gsi_flags,
                gpio_pin,
            });
        }
        crate::println!(
            "hid-i2c: PNP0C50 esclavo {slave:#04x} gsi={gsi:?} gpio={gpio_pin:?}; {} controlador(es) I2C",
            mmios.len()
        );
    }
    out
}

pub fn discover_amd_gpio() -> Option<AmdGpioBlock> {
    let mut found = None;
    acpi::each_table(b"DSDT", |aml| {
        if found.is_none() {
            found = scan_amd_gpio(aml);
        }
    });
    acpi::each_table(b"SSDT", |aml| {
        if found.is_none() {
            found = scan_amd_gpio(aml);
        }
    });
    if let Some(b) = found {
        crate::klogln!(
            "amd-gpio: AMDI0030 mmio={:#x} len={} gsi={:?}",
            b.mmio,
            b.len,
            b.gsi
        );
    }
    found
}

fn scan_amd_gpio(aml: &[u8]) -> Option<AmdGpioBlock> {
    for (off, _) in find_all(aml, b"AMDI0030") {
        let (mmio, len) = scan_memory32fixed(aml, off)?;
        let (gsi, gsi_flags) = scan_extended_interrupt(aml, off)
            .map(|(g, f)| (Some(g), f))
            .unwrap_or((None, 0));
        return Some(AmdGpioBlock {
            mmio,
            len,
            gsi,
            gsi_flags,
        });
    }
    None
}

/// `EISAID("PNP0C50")` en AML: DWordConst (0x0C) + 0x41 0xD0 0x0C 0x50.
const PNP0C50_EISAID: [u8; 5] = [0x0C, 0x41, 0xD0, 0x0C, 0x50];

fn scan_aml(
    aml: &[u8],
    mmios: &mut Vec<(u64, u32)>,
    slaves: &mut Vec<(u8, Option<u32>, u16, Option<u16>)>,
) {
    for (off, _) in find_all(aml, b"AMDI0010") {
        if let Some(pair) = scan_memory32fixed(aml, off) {
            if !mmios.iter().any(|m| m.0 == pair.0) {
                mmios.push(pair);
            }
        }
    }
    // El touchpad suele llevar un `_HID` del fabricante (ELAN…, ASUE…) y
    // `PNP0C50` sólo en `_CID`, como cadena o como `EISAID` (DWordConst).
    let cadenas = find_all(aml, b"PNP0C50");
    let eisaid = find_all(aml, &PNP0C50_EISAID);
    for (off, _) in cadenas.into_iter().chain(eisaid) {
        if let Some(slave) = scan_i2c_slave(aml, off) {
            let (gsi, gsi_flags) = scan_extended_interrupt(aml, off)
                .map(|(g, f)| (Some(g), f))
                .unwrap_or((None, 0));
            let gpio_pin = scan_gpio_int_pin(aml, off);
            if !slaves.iter().any(|s| s.0 == slave) {
                slaves.push((slave, gsi, gsi_flags, gpio_pin));
            }
        }
    }
}

/// Sin touchpad I2C: qué hay en las tablas que se le parezca. Sirve para
/// saber qué identificador usa una placa que no conocemos (otro controlador
/// I2C, o el touchpad declarado con `EISAID` en vez de con cadena).
fn informar_candidatos() {
    let mut ids: Vec<[u8; 8]> = Vec::new();
    let mut pnp_cadena = 0usize;
    let mut pnp_eisaid = 0usize;
    let mut otros: Vec<&'static str> = Vec::new();
    const PREFIJOS: [&[u8]; 3] = [b"AMDI", b"AMD0", b"INT3"];
    const NOMBRES: [&str; 8] = ["ACPI0C50", "MSFT0001", "ELAN", "SYNA", "ASUE", "FTCS", "PIXA", "GXTP"];
    let mut mirar = |aml: &[u8]| {
        for pre in PREFIJOS {
            for (off, _) in find_all(aml, pre) {
                if let Some(id) = aml.get(off..off + 8) {
                    if id.iter().all(|b| b.is_ascii_alphanumeric()) {
                        let mut k = [0u8; 8];
                        k.copy_from_slice(id);
                        if !ids.contains(&k) && ids.len() < 24 {
                            ids.push(k);
                        }
                    }
                }
            }
        }
        pnp_cadena += find_all(aml, b"PNP0C50").len();
        pnp_eisaid += find_all(aml, &PNP0C50_EISAID).len();
        for n in NOMBRES {
            if !otros.contains(&n) && !find_all(aml, n.as_bytes()).is_empty() {
                otros.push(n);
            }
        }
    };
    acpi::each_table(b"DSDT", &mut mirar);
    acpi::each_table(b"SSDT", &mut mirar);
    let mut lista = alloc::string::String::new();
    for id in &ids {
        lista.push(' ');
        lista.push_str(core::str::from_utf8(id).unwrap_or("?"));
    }
    crate::klogln!(
        "hid-i2c: candidatos ACPI:{lista} | PNP0C50 cadena={pnp_cadena} eisaid={pnp_eisaid} | otros={otros:?}"
    );
}

fn find_all(hay: &[u8], needle: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            out.push((i, i));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Tramo del `Device` de AML que contiene `off`: de su `DeviceOp` al final
/// del paquete. Los recursos de un dispositivo (`_CRS`) están dentro de él, y
/// buscarlos en una ventana fija a partir del `_HID` acababa leyendo los del
/// dispositivo siguiente. Si no se encuentra, una ventana de respaldo.
fn rango_device(aml: &[u8], off: usize) -> (usize, usize) {
    let tope = off.saturating_sub(256 * 1024);
    let mut i = off.min(aml.len().saturating_sub(2));
    while i > tope {
        i -= 1;
        // DeviceOp = ExtOpPrefix (0x5B) + 0x82, PkgLength, NameString.
        if aml[i] != 0x5B || aml.get(i + 1) != Some(&0x82) {
            continue;
        }
        let Some((len, n)) = pkg_length(aml, i + 2) else {
            continue;
        };
        let fin = i + 2 + len;
        let nombre = i + 2 + n;
        let nombre_ok = aml
            .get(nombre..nombre + 4)
            .is_some_and(|s| s.iter().all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'));
        if nombre_ok && fin > off && fin <= aml.len() {
            return (i, fin);
        }
    }
    (off, (off + 2048).min(aml.len()))
}

/// PkgLength de AML en `at`: (longitud, bytes que ocupa la propia codificación).
/// La longitud cuenta desde el primer byte de la codificación.
fn pkg_length(aml: &[u8], at: usize) -> Option<(usize, usize)> {
    let b0 = *aml.get(at)? as usize;
    let extra = b0 >> 6;
    if extra == 0 {
        return Some((b0 & 0x3F, 1));
    }
    let mut len = b0 & 0x0F;
    for k in 0..extra {
        len |= (*aml.get(at + 1 + k)? as usize) << (4 + 8 * k);
    }
    Some((len, 1 + extra))
}

/// Descriptores grandes de recursos (ACPI 6.4 §6.4.3) con etiqueta `tag`
/// dentro del `Device` que contiene `from`: `(inicio, longitud del cuerpo)`.
fn descriptores(aml: &[u8], from: usize, tag: u8) -> impl Iterator<Item = (usize, usize)> + '_ {
    let (ini, fin) = rango_device(aml, from);
    (ini..fin.saturating_sub(3)).filter_map(move |i| {
        if aml[i] != tag {
            return None;
        }
        let len = u16::from_le_bytes([aml[i + 1], aml[i + 2]]) as usize;
        (i + 3 + len <= fin).then_some((i, len))
    })
}

/// Memory32Fixed (0x86, longitud 9): información, base y tamaño.
fn scan_memory32fixed(aml: &[u8], from: usize) -> Option<(u64, u32)> {
    descriptores(aml, from, 0x86)
        .filter(|&(_, len)| len == 9)
        .find_map(|(i, _)| {
            let addr = u32::from_le_bytes(aml[i + 4..i + 8].try_into().ok()?);
            let len = u32::from_le_bytes(aml[i + 8..i + 12].try_into().ok()?);
            (addr != 0 && len >= 0x1000).then_some((u64::from(addr), len))
        })
}

/// Interrupt extendido (0x89): flags en el byte 3, nº de entradas en el 4 y
/// el primer GSI en el 5.
fn scan_extended_interrupt(aml: &[u8], from: usize) -> Option<(u32, u16)> {
    descriptores(aml, from, 0x89)
        .filter(|&(i, len)| len >= 6 && aml[i + 4] >= 1)
        .find_map(|(i, _)| {
            let flags = aml[i + 3] as u16;
            let gsi = u32::from_le_bytes(aml[i + 5..i + 9].try_into().ok()?);
            (gsi != 0 && gsi < 4096).then_some((gsi, flags))
        })
}

/// GpioInt (0x8C, tipo de conexión 0 en el byte 4): primer pin. El
/// desplazamiento de la tabla de pines (bytes 14-15) cuenta desde el inicio
/// del descriptor.
fn scan_gpio_int_pin(aml: &[u8], from: usize) -> Option<u16> {
    descriptores(aml, from, 0x8C)
        .filter(|&(i, len)| len >= 20 && aml[i + 4] == 0)
        .find_map(|(i, len)| {
            let tabla = u16::from_le_bytes([aml[i + 14], aml[i + 15]]) as usize;
            if tabla < 23 || tabla + 2 > len + 3 {
                return None;
            }
            let pin = u16::from_le_bytes([aml[i + tabla], aml[i + tabla + 1]]);
            (pin < 512).then_some(pin)
        })
}

/// I2cSerialBus (0x8E, tipo de bus 1 en el byte 5): la dirección del esclavo
/// va en los bytes 16-17 (7 bits salvo el modo de 10, que no se admite).
fn scan_i2c_slave(aml: &[u8], from: usize) -> Option<u8> {
    descriptores(aml, from, 0x8E)
        .filter(|&(i, len)| len >= 15 && aml[i + 5] == 1)
        .find_map(|(i, _)| {
            let diez_bits = aml[i + 7] & 1 != 0;
            let tipo_len = u16::from_le_bytes([aml[i + 10], aml[i + 11]]);
            let slave = u16::from_le_bytes([aml[i + 16], aml[i + 17]]);
            (!diez_bits && tipo_len >= 6 && (0x08..0x78).contains(&slave)).then_some(slave as u8)
        })
}

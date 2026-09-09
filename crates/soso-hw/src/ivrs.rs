//! IVRS — «I/O Virtualization Reporting Structure» (IOMMU AMD-Vi).
//!
//! Referencia: `struct ivhd_header` en
//! `lxdde/linux/drivers/iommu/amd/init.c:102` y el recorrido de
//! `init_iommu_all()`. Aquí sólo se localizan los IOMMU y su BAR MMIO; el
//! kernel es quien lee y escribe el registro de control.
//!
//! Distribución de la tabla:
//!
//! ```text
//! 0   cabecera SDT ACPI (36 B; longitud total en el offset 4)
//! 36  IVinfo (u32)
//! 40  reservado (u64)
//! 48  bloques IVHD encadenados por su propio campo `length`
//! ```

/// Un IOMMU descrito por un bloque IVHD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iommu {
    /// Tipo del bloque IVHD que lo describió (0x10, 0x11 o 0x40).
    pub ivhd_type: u8,
    /// BDF del propio IOMMU dentro del segmento.
    pub devid: u16,
    /// Offset de su capability en el espacio de configuración.
    pub cap_ptr: u16,
    /// Base física del bloque de registros MMIO.
    pub mmio_phys: u64,
    pub pci_seg: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IvrsError {
    /// El slice no llega ni a la cabecera SDT.
    Corta,
    /// La firma no es `IVRS`.
    Firma,
    /// El campo `length` de la cabecera no cabe en el slice recibido.
    LongitudIncoherente,
}

fn u16_le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_le(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

/// Cabecera IVHD mínima: hasta `efr_attr` son 24 bytes.
const IVHD_MIN: usize = 24;
/// Primer bloque IVHD tras cabecera SDT + IVinfo + reservado.
const IVHD_INICIO: usize = 48;

/// Localiza los IOMMU descritos en `tabla` y los escribe en `out`.
///
/// Devuelve cuántos se han escrito. Un mismo IOMMU puede aparecer en varios
/// bloques (tipos 0x10 y 0x11/0x40 describen el mismo hardware con más campos),
/// así que se deduplica por `mmio_phys` quedándose con el primero visto —
/// igual que Linux, que procesa un único tipo de IVHD por arranque.
///
/// Si hay más IOMMU que hueco en `out`, se escriben los que quepan; el valor
/// devuelto nunca supera `out.len()`.
pub fn parse(tabla: &[u8], out: &mut [Iommu]) -> Result<usize, IvrsError> {
    if tabla.len() < 36 {
        return Err(IvrsError::Corta);
    }
    if &tabla[0..4] != b"IVRS" {
        return Err(IvrsError::Firma);
    }
    let len = u32_le(tabla, 4) as usize;
    if len > tabla.len() || len < IVHD_INICIO {
        return Err(IvrsError::LongitudIncoherente);
    }

    let mut n = 0usize;
    let mut off = IVHD_INICIO;
    while off + IVHD_MIN <= len {
        let ivhd_type = tabla[off];
        let blen = u16_le(tabla, off + 2) as usize;
        // Un bloque que no avanza dejaría el recorrido girando para siempre:
        // una tabla corrupta no puede colgar el arranque.
        if blen < IVHD_MIN || off + blen > len {
            break;
        }
        // Sólo los tipos conocidos tienen esta distribución de campos.
        if matches!(ivhd_type, 0x10 | 0x11 | 0x40) {
            let iommu = Iommu {
                ivhd_type,
                devid: u16_le(tabla, off + 4),
                cap_ptr: u16_le(tabla, off + 6),
                mmio_phys: u64_le(tabla, off + 8),
                pci_seg: u16_le(tabla, off + 16),
            };
            let repetido = out[..n].iter().any(|i| i.mmio_phys == iommu.mmio_phys);
            if !repetido && iommu.mmio_phys != 0 {
                if n == out.len() {
                    return Ok(n);
                }
                out[n] = iommu;
                n += 1;
            }
        }
        off += blen;
    }
    Ok(n)
}

// ---- Registros MMIO del IOMMU ----
//
// `lxdde/linux/drivers/iommu/amd/amd_iommu_types.h:57` y siguientes.

/// Offset del registro de control (64 bits) dentro del MMIO del IOMMU.
pub const MMIO_CONTROL_OFFSET: u64 = 0x0018;

/// Bits del registro de control usados al apagar el IOMMU.
/// Orden y selección tomados de `iommu_disable()` (`init.c:470`).
pub const CONTROL_IOMMU_EN: u32 = 0;
pub const CONTROL_EVT_LOG_EN: u32 = 2;
pub const CONTROL_EVT_INT_EN: u32 = 3;
pub const CONTROL_CMDBUF_EN: u32 = 12;
pub const CONTROL_PPRLOG_EN: u32 = 13;
pub const CONTROL_PPRINT_EN: u32 = 14;
pub const CONTROL_GALOG_EN: u32 = 28;
pub const CONTROL_GAINT_EN: u32 = 29;

/// Bits que `iommu_disable()` limpia antes de quitar `IOMMU_EN`, en ese orden.
pub const APAGADO_ORDENADO: [u32; 8] = [
    CONTROL_CMDBUF_EN,
    CONTROL_EVT_INT_EN,
    CONTROL_EVT_LOG_EN,
    CONTROL_GALOG_EN,
    CONTROL_GAINT_EN,
    CONTROL_PPRLOG_EN,
    CONTROL_PPRINT_EN,
    CONTROL_IOMMU_EN,
];

/// ¿Está el IOMMU traduciendo ahora mismo?
pub fn habilitado(control: u64) -> bool {
    control & (1u64 << CONTROL_IOMMU_EN) != 0
}

/// Valor del registro de control tras aplicar el apagado ordenado.
pub fn control_apagado(control: u64) -> u64 {
    let mut v = control;
    for bit in APAGADO_ORDENADO {
        v &= !(1u64 << bit);
    }
    v
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    extern crate std;

    use super::*;

    /// Construye una IVRS sintética con los IVHD indicados como
    /// `(tipo, devid, cap_ptr, mmio_phys)`.
    fn ivrs(bloques: &[(u8, u16, u16, u64)]) -> alloc::vec::Vec<u8> {
        let mut t = alloc::vec![0u8; IVHD_INICIO];
        t[0..4].copy_from_slice(b"IVRS");
        for &(tipo, devid, cap, mmio) in bloques {
            let blen: u16 = if tipo == 0x10 { 24 } else { 40 };
            let mut b = alloc::vec![0u8; blen as usize];
            b[0] = tipo;
            b[2..4].copy_from_slice(&blen.to_le_bytes());
            b[4..6].copy_from_slice(&devid.to_le_bytes());
            b[6..8].copy_from_slice(&cap.to_le_bytes());
            b[8..16].copy_from_slice(&mmio.to_le_bytes());
            t.extend_from_slice(&b);
        }
        let len = t.len() as u32;
        t[4..8].copy_from_slice(&len.to_le_bytes());
        t
    }

    #[test]
    fn un_iommu_tipo_10() {
        let t = ivrs(&[(0x10, 0x0002, 0x40, 0xfeb8_0000)]);
        let mut out = [Iommu {
            ivhd_type: 0,
            devid: 0,
            cap_ptr: 0,
            mmio_phys: 0,
            pci_seg: 0,
        }; 4];
        let n = parse(&t, &mut out).unwrap();
        assert_eq!(n, 1);
        assert_eq!(out[0].mmio_phys, 0xfeb8_0000);
        assert_eq!(out[0].devid, 0x0002);
        assert_eq!(out[0].cap_ptr, 0x40);
        assert_eq!(out[0].ivhd_type, 0x10);
    }

    #[test]
    fn tipos_10_y_11_del_mismo_iommu_no_se_duplican() {
        // Es el caso real: el firmware publica el mismo IOMMU dos veces.
        let t = ivrs(&[
            (0x10, 0x0002, 0x40, 0xfeb8_0000),
            (0x11, 0x0002, 0x40, 0xfeb8_0000),
        ]);
        let mut out = [Iommu {
            ivhd_type: 0,
            devid: 0,
            cap_ptr: 0,
            mmio_phys: 0,
            pci_seg: 0,
        }; 4];
        assert_eq!(parse(&t, &mut out).unwrap(), 1);
        assert_eq!(out[0].ivhd_type, 0x10, "gana el primero visto");
    }

    #[test]
    fn dos_iommu_distintos() {
        let t = ivrs(&[
            (0x10, 0x0002, 0x40, 0xfeb8_0000),
            (0x10, 0x000a, 0x40, 0xfec0_0000),
        ]);
        let mut out = [Iommu {
            ivhd_type: 0,
            devid: 0,
            cap_ptr: 0,
            mmio_phys: 0,
            pci_seg: 0,
        }; 4];
        assert_eq!(parse(&t, &mut out).unwrap(), 2);
        assert_eq!(out[1].mmio_phys, 0xfec0_0000);
    }

    #[test]
    fn bloque_de_longitud_cero_no_cuelga() {
        let mut t = ivrs(&[(0x10, 0x0002, 0x40, 0xfeb8_0000)]);
        // Longitud 0 en el bloque: sin la guarda, el recorrido no avanzaría.
        t[IVHD_INICIO + 2] = 0;
        t[IVHD_INICIO + 3] = 0;
        let mut out = [Iommu {
            ivhd_type: 0,
            devid: 0,
            cap_ptr: 0,
            mmio_phys: 0,
            pci_seg: 0,
        }; 4];
        assert_eq!(parse(&t, &mut out).unwrap(), 0);
    }

    #[test]
    fn bloque_que_desborda_la_tabla_se_ignora() {
        let mut t = ivrs(&[(0x10, 0x0002, 0x40, 0xfeb8_0000)]);
        let grande: u16 = 4096;
        t[IVHD_INICIO + 2..IVHD_INICIO + 4].copy_from_slice(&grande.to_le_bytes());
        let mut out = [Iommu {
            ivhd_type: 0,
            devid: 0,
            cap_ptr: 0,
            mmio_phys: 0,
            pci_seg: 0,
        }; 4];
        assert_eq!(parse(&t, &mut out).unwrap(), 0);
    }

    #[test]
    fn firma_y_longitud_se_validan() {
        let mut t = ivrs(&[(0x10, 2, 0x40, 0xfeb8_0000)]);
        let mut out = [Iommu {
            ivhd_type: 0,
            devid: 0,
            cap_ptr: 0,
            mmio_phys: 0,
            pci_seg: 0,
        }; 4];
        t[0] = b'X';
        assert_eq!(parse(&t, &mut out), Err(IvrsError::Firma));
        t[0] = b'I';
        let mentira = (t.len() as u32) + 64;
        t[4..8].copy_from_slice(&mentira.to_le_bytes());
        assert_eq!(parse(&t, &mut out), Err(IvrsError::LongitudIncoherente));
        assert_eq!(parse(&t[..8], &mut out), Err(IvrsError::Corta));
    }

    #[test]
    fn out_pequeno_no_desborda() {
        let t = ivrs(&[
            (0x10, 0x0002, 0x40, 0xfeb8_0000),
            (0x10, 0x000a, 0x40, 0xfec0_0000),
            (0x10, 0x0012, 0x40, 0xfed0_0000),
        ]);
        let mut out = [Iommu {
            ivhd_type: 0,
            devid: 0,
            cap_ptr: 0,
            mmio_phys: 0,
            pci_seg: 0,
        }; 2];
        assert_eq!(parse(&t, &mut out).unwrap(), 2);
    }

    #[test]
    fn apagado_limpia_los_bits_de_iommu_disable() {
        // Control con IOMMU_EN + cmdbuf + event log + GA log encendidos.
        let ctrl = (1 << CONTROL_IOMMU_EN)
            | (1 << CONTROL_CMDBUF_EN)
            | (1 << CONTROL_EVT_LOG_EN)
            | (1 << CONTROL_GALOG_EN)
            | (1 << 10); // COHERENT_EN: no lo toca iommu_disable()
        assert!(habilitado(ctrl));
        let apagado = control_apagado(ctrl);
        assert!(!habilitado(apagado));
        assert_eq!(apagado, 1 << 10, "sólo se limpian los bits de iommu_disable");
    }
}

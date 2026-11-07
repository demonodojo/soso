//! XID de `TxConfig` según Linux `rtl8169_get_mac_version` (r8169_main.c).
//!
//! El 8125 (PCI `10ec:8125`) es VER_61 o VER_63. No es un 8168H.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RtlFamily {
    Rtl8168h,
    Rtl8125a,
    Rtl8125b,
    Other(u16),
}

/// IDs PCI que reclama el driver. `0x8125` es el RTL8125.
pub const PCI_DEVICE_IDS: &[u16] = &[0x8168, 0x8161, 0x8162, 0x8167, 0x8136, 0x8125];

/// `xid_raw` es `TxConfig >> 20`, ya enmascarado con `0xfcf` o sin enmascarar:
/// las máscaras de Linux (`0x7cf`) no miran los bits que `0xfcf` tira.
pub fn family(xid_raw: u16) -> RtlFamily {
    const TABLE: &[(u16, u16, RtlFamily)] = &[
        (0x7cf, 0x641, RtlFamily::Rtl8125b),
        (0x7cf, 0x609, RtlFamily::Rtl8125a),
        (0x7cf, 0x541, RtlFamily::Rtl8168h),
    ];
    for &(mask, val, kind) in TABLE {
        if xid_raw & mask == val {
            return kind;
        }
    }
    let masked = xid_raw & 0xfcf;
    if masked == 0x540 || masked == 0x541 {
        return RtlFamily::Rtl8168h;
    }
    RtlFamily::Other(masked)
}

pub fn is_8125(xid_raw: u16) -> bool {
    matches!(family(xid_raw), RtlFamily::Rtl8125a | RtlFamily::Rtl8125b)
}

pub fn is_8168h(xid_raw: u16) -> bool {
    matches!(family(xid_raw), RtlFamily::Rtl8168h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_8125_no_entra_por_8168h() {
        assert!(PCI_DEVICE_IDS.contains(&0x8125));
        assert_eq!(family(0x609), RtlFamily::Rtl8125a);
        assert_eq!(family(0x641), RtlFamily::Rtl8125b);
        assert!(is_8125(0x609));
        assert!(is_8125(0x641));
        assert!(!is_8168h(0x609));
        assert!(!is_8168h(0x641));
        assert_eq!(family(0x541), RtlFamily::Rtl8168h);
        assert_eq!(family(0x540), RtlFamily::Rtl8168h);
        assert!(is_8168h(0x541));
        assert!(!is_8125(0x541));
    }
}

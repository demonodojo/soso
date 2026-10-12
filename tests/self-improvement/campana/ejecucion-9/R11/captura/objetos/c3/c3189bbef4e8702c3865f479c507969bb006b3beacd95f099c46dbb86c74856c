//! Rotación de la consola framebuffer.
//!
//! El panel de la Steam Deck es **800×1280 vertical nativo** montado girado:
//! el GOP entrega ese buffer tal cual y la consola sale de lado. Linux lo trata
//! con un quirk DMI (`lcd800x1280_rightside_up`,
//! `drm_panel_orientation_quirks.c:108`) que marca el panel como
//! `DRM_MODE_PANEL_ORIENTATION_RIGHT_UP`, y `drm_client_modeset.c:926` lo
//! traduce a `DRM_MODE_ROTATE_270`.
//!
//! Se adopta la misma convención que DRM: **los grados son antihorarios**
//! (`drm_blend.c`, «Rotation is the specified amount in degrees in counter
//! clockwise direction»). Así, `R270` es la rotación de la Deck sin traducir
//! nada por el camino.
//!
//! Vocabulario: *lógico* es lo que ve la consola (donde el texto corre a lo
//! ancho); *físico* es el buffer del GOP.

/// Rotación aplicada a la imagen lógica al volcarla al framebuffer físico.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rot {
    #[default]
    R0,
    /// 90° antihorario.
    R90,
    R180,
    /// 270° antihorario (≡ 90° horario). La de la Steam Deck.
    R270,
}

impl Rot {
    /// `"0" | "90" | "180" | "270" | "auto"` → rotación. `auto` decide por la
    /// forma del panel y necesita las dimensiones físicas.
    pub fn parse(s: &str, phys_w: usize, phys_h: usize) -> Option<Rot> {
        match s.trim() {
            "0" | "normal" => Some(Rot::R0),
            "90" => Some(Rot::R90),
            "180" => Some(Rot::R180),
            "270" => Some(Rot::R270),
            "auto" => Some(Rot::automatica(phys_w, phys_h)),
            _ => None,
        }
    }

    /// Un panel más alto que ancho es un panel montado girado: se asume el
    /// caso de la Deck (`RIGHT_UP` → 270°). Es la convención de Linux para
    /// este formato, pero el sentido real sólo lo confirma la placa; por eso
    /// existe la anulación explícita.
    pub fn automatica(phys_w: usize, phys_h: usize) -> Rot {
        if phys_h > phys_w { Rot::R270 } else { Rot::R0 }
    }

    /// ¿Intercambia ancho y alto?
    pub fn traspone(self) -> bool {
        matches!(self, Rot::R90 | Rot::R270)
    }

    /// ¿Se puede volcar el buffer con un `memcpy` lineal?
    ///
    /// Sólo sin rotación: con 90/270 una línea lógica es una columna física, y
    /// con 180 el orden se invierte. El camino rápido de scroll de `fb.rs`
    /// depende de esto.
    pub fn lineal(self) -> bool {
        matches!(self, Rot::R0)
    }

    pub fn grados(self) -> u16 {
        match self {
            Rot::R0 => 0,
            Rot::R90 => 90,
            Rot::R180 => 180,
            Rot::R270 => 270,
        }
    }
}

/// Dimensiones lógicas de la consola para un panel físico dado.
pub fn dim_logica(rot: Rot, phys_w: usize, phys_h: usize) -> (usize, usize) {
    if rot.traspone() {
        (phys_h, phys_w)
    } else {
        (phys_w, phys_h)
    }
}

/// Píxel físico donde cae el píxel lógico `(x, y)`.
///
/// Devuelve `None` si `(x, y)` cae fuera de la vista lógica, para que quien
/// pinta no tenga que replicar la aritmética de los límites.
pub fn a_fisico(
    rot: Rot,
    phys_w: usize,
    phys_h: usize,
    x: usize,
    y: usize,
) -> Option<(usize, usize)> {
    let (lw, lh) = dim_logica(rot, phys_w, phys_h);
    if x >= lw || y >= lh {
        return None;
    }
    Some(match rot {
        Rot::R0 => (x, y),
        // 90° antihorario: la esquina superior izquierda baja a la inferior izquierda.
        Rot::R90 => (y, phys_h - 1 - x),
        Rot::R180 => (phys_w - 1 - x, phys_h - 1 - y),
        // 270° antihorario ≡ 90° horario: la superior izquierda va a la superior derecha.
        Rot::R270 => (phys_w - 1 - y, x),
    })
}

/// Inversa de [`a_fisico`], para traducir coordenadas que llegan en píxeles
/// físicos (p. ej. un puntero) a la vista lógica.
pub fn a_logico(
    rot: Rot,
    phys_w: usize,
    phys_h: usize,
    px: usize,
    py: usize,
) -> Option<(usize, usize)> {
    if px >= phys_w || py >= phys_h {
        return None;
    }
    Some(match rot {
        Rot::R0 => (px, py),
        Rot::R90 => (phys_h - 1 - py, px),
        Rot::R180 => (phys_w - 1 - px, phys_h - 1 - py),
        Rot::R270 => (py, phys_w - 1 - px),
    })
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    const DECK_W: usize = 800;
    const DECK_H: usize = 1280;

    #[test]
    fn la_deck_da_una_consola_apaisada() {
        let rot = Rot::automatica(DECK_W, DECK_H);
        assert_eq!(rot, Rot::R270, "Linux usa ROTATE_270 para este panel");
        assert_eq!(dim_logica(rot, DECK_W, DECK_H), (1280, 800));
    }

    #[test]
    fn un_panel_apaisado_no_se_rota() {
        assert_eq!(Rot::automatica(1920, 1080), Rot::R0);
        assert_eq!(dim_logica(Rot::R0, 1920, 1080), (1920, 1080));
    }

    #[test]
    fn r270_lleva_el_origen_a_la_esquina_superior_derecha() {
        // Físico 2×3 → lógico 3×2.
        assert_eq!(a_fisico(Rot::R270, 2, 3, 0, 0), Some((1, 0)));
        // Avanzar en x (a lo ancho del texto) baja por el panel.
        assert_eq!(a_fisico(Rot::R270, 2, 3, 1, 0), Some((1, 1)));
        assert_eq!(a_fisico(Rot::R270, 2, 3, 2, 0), Some((1, 2)));
        // Avanzar en y (línea siguiente) se mueve hacia la izquierda del panel.
        assert_eq!(a_fisico(Rot::R270, 2, 3, 0, 1), Some((0, 0)));
    }

    #[test]
    fn r90_es_el_sentido_contrario_a_r270() {
        assert_eq!(a_fisico(Rot::R90, 2, 3, 0, 0), Some((0, 2)));
        assert_eq!(a_fisico(Rot::R90, 2, 3, 2, 0), Some((0, 0)));
        assert_eq!(a_fisico(Rot::R90, 2, 3, 0, 1), Some((1, 2)));
    }

    #[test]
    fn r180_invierte_ambos_ejes() {
        assert_eq!(a_fisico(Rot::R180, 2, 3, 0, 0), Some((1, 2)));
        assert_eq!(a_fisico(Rot::R180, 2, 3, 1, 2), Some((0, 0)));
    }

    #[test]
    fn cada_rotacion_es_una_biyeccion() {
        // Ningún píxel lógico se pierde ni dos caen en el mismo sitio: si esto
        // falla, la consola pinta encima de sí misma en alguna esquina.
        for rot in [Rot::R0, Rot::R90, Rot::R180, Rot::R270] {
            let (pw, ph) = (5usize, 3usize);
            let (lw, lh) = dim_logica(rot, pw, ph);
            let mut visto = std::collections::BTreeSet::new();
            for y in 0..lh {
                for x in 0..lw {
                    let p = a_fisico(rot, pw, ph, x, y).expect("dentro de la vista");
                    assert!(p.0 < pw && p.1 < ph, "{rot:?} sale del panel: {p:?}");
                    assert!(visto.insert(p), "{rot:?} repite el físico {p:?}");
                }
            }
            assert_eq!(visto.len(), pw * ph, "{rot:?} deja píxeles sin cubrir");
        }
    }

    #[test]
    fn a_logico_deshace_a_fisico() {
        for rot in [Rot::R0, Rot::R90, Rot::R180, Rot::R270] {
            let (pw, ph) = (5usize, 3usize);
            let (lw, lh) = dim_logica(rot, pw, ph);
            for y in 0..lh {
                for x in 0..lw {
                    let (px, py) = a_fisico(rot, pw, ph, x, y).unwrap();
                    assert_eq!(a_logico(rot, pw, ph, px, py), Some((x, y)), "{rot:?}");
                }
            }
        }
    }

    #[test]
    fn fuera_de_la_vista_es_none() {
        let (pw, ph) = (2usize, 3usize);
        // Lógico 3×2 con R270: (3,0) y (0,2) están fuera.
        assert_eq!(a_fisico(Rot::R270, pw, ph, 3, 0), None);
        assert_eq!(a_fisico(Rot::R270, pw, ph, 0, 2), None);
        assert_eq!(a_logico(Rot::R270, pw, ph, 2, 0), None);
    }

    #[test]
    fn solo_r0_admite_el_camino_rapido() {
        assert!(Rot::R0.lineal());
        for rot in [Rot::R90, Rot::R180, Rot::R270] {
            assert!(!rot.lineal(), "{rot:?} no puede hacer memcpy de líneas");
        }
    }

    #[test]
    fn parse_acepta_grados_y_auto() {
        assert_eq!(Rot::parse("0", 800, 1280), Some(Rot::R0));
        assert_eq!(Rot::parse(" 270 ", 800, 1280), Some(Rot::R270));
        assert_eq!(Rot::parse("auto", 800, 1280), Some(Rot::R270));
        assert_eq!(Rot::parse("auto", 1920, 1080), Some(Rot::R0));
        assert_eq!(Rot::parse("45", 800, 1280), None);
    }
}

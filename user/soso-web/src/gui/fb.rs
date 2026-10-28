//! Superficie gráfica: ventana vía soso-wm o framebuffer a pantalla completa.

use libsoso::sys;
use libsoso::ventana::{Evento, Superficie};
use soso_abi::FbInfo;

pub struct Framebuffer {
    pub info: FbInfo,
    inner: Superficie,
}

impl Framebuffer {
    pub fn open() -> Result<Self, i64> {
        Self::open_size(1024, 768, "")
    }

    pub fn open_with_title(w: u32, h: u32, titulo: &str) -> Result<Self, i64> {
        let inner = Superficie::abrir_con_titulo(w, h, titulo)?;
        let info = inner.fb_info();
        Ok(Self { info, inner })
    }

    /// Tamaño cliente sugerido (~80 % del GOP, mínimo 640×480).
    pub fn tamaño_inicial_cliente() -> (u32, u32) {
        let mut info = FbInfo::default();
        if sys::fb_info(&mut info) >= 0 && info.present != 0 && info.width > 0 && info.height > 0 {
            let sw = info.width;
            let sh = info.height;
            let w = (sw * 80 / 100).max(640).min(sw);
            let h = (sh * 80 / 100).max(480).min(sh);
            (w, h)
        } else {
            (1024, 768)
        }
    }

    pub fn enviar_titulo_wm(&self, titulo: &str) -> Result<(), i64> {
        self.inner.enviar_titulo(titulo)
    }

    fn open_size(w: u32, h: u32, titulo: &str) -> Result<Self, i64> {
        Self::open_with_title(w, h, titulo)
    }

    pub fn back_mut(&mut self) -> &mut [u8] {
        self.inner.back_mut()
    }

    pub fn es_wm(&self) -> bool {
        self.inner.es_wm()
    }

    pub(crate) fn inner_mut(&mut self) -> &mut Superficie {
        &mut self.inner
    }

    pub fn present(&self) -> Result<(), i64> {
        self.inner.commit()
    }

    pub fn poll_wm_event(&self) -> Result<Option<Evento>, i64> {
        self.inner.poll_event()
    }
}

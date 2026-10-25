//! Superficie gráfica: ventana vía soso-wm o framebuffer a pantalla completa.

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

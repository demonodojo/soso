//! Ventana gráfica vía soso-wm o, si no hay WM, pantalla completa.

use crate::abi::{self, SockAddr};
use crate::sys;

const WM_PORT: u16 = 7430;
const MSG_LEN: usize = 32;

const OP_DANIO: u32 = 3;
const OP_TITULO: u32 = 4;
const OP_CERRAR_CLI: u32 = 5;
const OP_VENTANA: u32 = 16;
const OP_TECLA: u32 = 17;
const OP_RATON: u32 = 18;
const OP_FOCO: u32 = 19;
const OP_REDIM: u32 = 20;
const OP_CERRAR_WM: u32 = 21;
const OP_TEMA: u32 = 23;
const OP_ESCALA: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evento {
    Tecla { key: u32, mods: u32 },
    Raton { x: i32, y: i32, botones: u32 },
    Foco,
    Redimensionar { w: u32, h: u32, shm_id: u64 },
    CerrarPedido,
    Tema { claro: bool },
    Escala { nivel: u32, glifo_w: u32, glifo_h: u32 },
}

pub enum Superficie {
    Wm {
        sock: u64,
        shm_id: u64,
        ptr: u64,
        len: usize,
        width: u32,
        height: u32,
        stride: u32,
        pixel_format: u8,
        bytes_per_pixel: u8,
        tema: u32,
        nivel: u32,
    },
    PantallaCompleta {
        fb: alloc::vec::Vec<u8>,
        info: abi::FbInfo,
    },
}

fn put_u32(buf: &mut [u8; MSG_LEN], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn u32_le(buf: &[u8; MSG_LEN], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

impl Superficie {
    pub fn abrir(w: u32, h: u32) -> Result<Self, i64> {
        if let Ok(s) = Self::abrir_wm(w, h) {
            return Ok(s);
        }
        Self::abrir_pantalla()
    }

    pub fn abrir_con_titulo(w: u32, h: u32, titulo: &str) -> Result<Self, i64> {
        if let Ok(s) = Self::abrir_wm_titulo(w, h, titulo) {
            return Ok(s);
        }
        Self::abrir_pantalla()
    }

    fn abrir_wm(w: u32, h: u32) -> Result<Self, i64> {
        Self::abrir_wm_titulo(w, h, "")
    }

    fn abrir_wm_titulo(w: u32, h: u32, titulo: &str) -> Result<Self, i64> {
        let addr = SockAddr {
            addr: [127, 0, 0, 1],
            port: WM_PORT.to_be(),
            _pad: 0,
        };
        let sock = sys::tcp_connect(&addr, 500);
        if sock < 0 {
            return Err(sock);
        }
        let mut hola = [0u8; MSG_LEN];
        put_u32(&mut hola, 0, 1);
        put_u32(&mut hola, 4, 1);
        if sys::write(sock as u64, &hola) != MSG_LEN as i64 {
            sys::close(sock as u64);
            return Err(-abi::EIO);
        }
        let mut crear = [0u8; MSG_LEN];
        put_u32(&mut crear, 0, 2);
        put_u32(&mut crear, 4, w);
        put_u32(&mut crear, 8, h);
        put_u32(&mut crear, 12, 16);
        let tlen = titulo.len().min(16);
        crear[16..16 + tlen].copy_from_slice(&titulo.as_bytes()[..tlen]);
        if sys::write(sock as u64, &crear) != MSG_LEN as i64 {
            sys::close(sock as u64);
            return Err(-abi::EIO);
        }
        let mut buf = [0u8; MSG_LEN];
        if sys::read(sock as u64, &mut buf) != MSG_LEN as i64 {
            sys::close(sock as u64);
            return Err(-abi::EIO);
        }
        if u32_le(&buf, 0) != OP_VENTANA {
            sys::close(sock as u64);
            return Err(-abi::EINVAL);
        }
        let shm_id = u32_le(&buf, 8) as u64;
        let ww = u32_le(&buf, 12);
        let hh = u32_le(&buf, 16);
        let fmt = u32_le(&buf, 20) as u8;
        let tema = u32_le(&buf, 24);
        let nivel = u32_le(&buf, 28);
        let va = sys::shm_map(shm_id)?;
        let stride = ww;
        let len = (stride as u64)
            .saturating_mul(hh as u64)
            .saturating_mul(4) as usize;
        Ok(Superficie::Wm {
            sock: sock as u64,
            shm_id,
            ptr: va,
            len,
            width: ww,
            height: hh,
            stride,
            pixel_format: fmt,
            bytes_per_pixel: 4,
            tema,
            nivel,
        })
    }

    pub fn tema_claro(&self) -> bool {
        match self {
            Superficie::Wm { tema, .. } => *tema != 0,
            Superficie::PantallaCompleta { .. } => false,
        }
    }

    pub fn nivel_inicial(&self) -> u32 {
        match self {
            Superficie::Wm { nivel, .. } => *nivel,
            Superficie::PantallaCompleta { .. } => 1,
        }
    }

    fn abrir_pantalla() -> Result<Self, i64> {
        let mut info = abi::FbInfo::default();
        if sys::fb_info(&mut info) < 0 || info.present == 0 {
            return Err(-abi::ENOTSUP);
        }
        sys::fb_set_mode(abi::FB_MODE_GRAPHICS)?;
        let bytes = info.byte_len as usize;
        Ok(Superficie::PantallaCompleta {
            fb: alloc::vec![0u8; bytes],
            info,
        })
    }

    pub fn fb_info(&self) -> abi::FbInfo {
        match self {
            Superficie::Wm {
                width,
                height,
                stride,
                pixel_format,
                bytes_per_pixel,
                len,
                ..
            } => abi::FbInfo {
                present: 1,
                pixel_format: *pixel_format,
                bytes_per_pixel: *bytes_per_pixel,
                _pad: 0,
                width: *width,
                height: *height,
                stride: *stride,
                byte_len: *len as u64,
            },
            Superficie::PantallaCompleta { info, .. } => *info,
        }
    }

    pub fn back_mut(&mut self) -> &mut [u8] {
        match self {
            Superficie::Wm { ptr, len, .. } => unsafe {
                core::slice::from_raw_parts_mut(*ptr as *mut u8, *len)
            },
            Superficie::PantallaCompleta { fb, .. } => fb.as_mut_slice(),
        }
    }

    pub fn es_wm(&self) -> bool {
        matches!(self, Superficie::Wm { .. })
    }

    pub fn aplicar_redimension(&mut self, w: u32, h: u32, new_shm: u64) -> Result<(), i64> {
        let Superficie::Wm {
            shm_id,
            ptr,
            len,
            width,
            height,
            stride,
            ..
        } = self
        else {
            return Ok(());
        };
        if *shm_id != 0 {
            let _ = sys::shm_free(*shm_id);
        }
        let va = sys::shm_map(new_shm)?;
        *shm_id = new_shm;
        *width = w;
        *height = h;
        *stride = w;
        *len = (w as u64 * h as u64 * 4) as usize;
        *ptr = va;
        Ok(())
    }

    /// Actualiza el título en la barra del WM (máx. 16 bytes).
    pub fn enviar_titulo(&self, titulo: &str) -> Result<(), i64> {
        let Superficie::Wm { sock, .. } = self else {
            return Ok(());
        };
        let mut m = [0u8; MSG_LEN];
        put_u32(&mut m, 0, OP_TITULO);
        put_u32(&mut m, 12, 16);
        let tlen = titulo.len().min(16);
        m[16..16 + tlen].copy_from_slice(&titulo.as_bytes()[..tlen]);
        if sys::write(*sock, &m) != MSG_LEN as i64 {
            return Err(-abi::EIO);
        }
        Ok(())
    }

    fn enviar_danio(&self, x: u32, y: u32, w: u32, h: u32) -> Result<(), i64> {
        let Superficie::Wm { sock, .. } = self else {
            return Ok(());
        };
        let mut m = [0u8; MSG_LEN];
        put_u32(&mut m, 0, OP_DANIO);
        put_u32(&mut m, 4, x);
        put_u32(&mut m, 8, y);
        put_u32(&mut m, 12, w);
        put_u32(&mut m, 16, h);
        if sys::write(*sock, &m) != MSG_LEN as i64 {
            return Err(-abi::EIO);
        }
        Ok(())
    }

    /// Notifica al WM que hay píxeles nuevos (modo ventana).
    pub fn commit_rect(&self, x: u32, y: u32, w: u32, h: u32) -> Result<(), i64> {
        match self {
            Superficie::Wm { .. } => self.enviar_danio(x, y, w, h),
            Superficie::PantallaCompleta { fb, .. } => {
                sys::fb_present(fb.as_ptr() as u64, fb.len() as u64)
            }
        }
    }

    pub fn commit(&self) -> Result<(), i64> {
        let info = self.fb_info();
        self.commit_rect(0, 0, info.width, info.height)
    }

    pub fn poll_event(&self) -> Result<Option<Evento>, i64> {
        let Superficie::Wm { sock, .. } = self else {
            return Ok(None);
        };
        let mut buf = [0u8; MSG_LEN];
        let n = sys::read_timeout(*sock, &mut buf, 1);
        if n == 0 {
            return Ok(None);
        }
        if n != MSG_LEN as i64 {
            return Ok(None);
        }
        match u32_le(&buf, 0) {
            OP_TECLA => Ok(Some(Evento::Tecla {
                key: u32_le(&buf, 4),
                mods: u32_le(&buf, 8),
            })),
            OP_RATON => Ok(Some(Evento::Raton {
                x: u32_le(&buf, 4) as i32,
                y: u32_le(&buf, 8) as i32,
                botones: u32_le(&buf, 12),
            })),
            OP_FOCO => Ok(Some(Evento::Foco)),
            OP_REDIM => {
                let shm_id = u32_le(&buf, 20) as u64;
                Ok(Some(Evento::Redimensionar {
                    w: u32_le(&buf, 4),
                    h: u32_le(&buf, 8),
                    shm_id,
                }))
            }
            OP_CERRAR_WM => Ok(Some(Evento::CerrarPedido)),
            OP_TEMA => Ok(Some(Evento::Tema {
                claro: u32_le(&buf, 4) != 0,
            })),
            OP_ESCALA => Ok(Some(Evento::Escala {
                nivel: u32_le(&buf, 4),
                glifo_w: u32_le(&buf, 8),
                glifo_h: u32_le(&buf, 12),
            })),
            _ => Ok(None),
        }
    }
}

impl Drop for Superficie {
    fn drop(&mut self) {
        match self {
            Superficie::Wm { sock, shm_id, .. } => {
                let mut m = [0u8; MSG_LEN];
                put_u32(&mut m, 0, OP_CERRAR_CLI);
                let _ = sys::write(*sock, &m);
                let _ = sys::shm_free(*shm_id);
                let _ = sys::close(*sock);
            }
            Superficie::PantallaCompleta { .. } => {
                let _ = sys::fb_set_mode(abi::FB_MODE_CONSOLE);
            }
        }
    }
}

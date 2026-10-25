//! Mensajes de 32 bytes en little-endian (mismo layout que `proto.sal`).

pub const MSG_LEN: usize = 32;

pub mod op {
    pub const HOLA: u32 = 1;
    pub const CREAR: u32 = 2;
    pub const DANIO: u32 = 3;
    pub const TITULO: u32 = 4;
    pub const CERRAR_CLI: u32 = 5;
    pub const VENTANA: u32 = 16;
    pub const TECLA: u32 = 17;
    pub const RATON: u32 = 18;
    pub const FOCO: u32 = 19;
    pub const REDIM: u32 = 20;
    pub const CERRAR_WM: u32 = 21;
    pub const FOTOGRAMA: u32 = 22;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Msg([u8; MSG_LEN]);

impl Msg {
    pub fn zero() -> Self {
        Self([0; MSG_LEN])
    }

    pub fn from_bytes(b: [u8; MSG_LEN]) -> Self {
        Self(b)
    }

    pub fn as_bytes(&self) -> &[u8; MSG_LEN] {
        &self.0
    }

    pub fn op(&self) -> u32 {
        u32::from_le_bytes(self.0[0..4].try_into().unwrap())
    }

    pub fn set_op(&mut self, op: u32) {
        self.0[0..4].copy_from_slice(&op.to_le_bytes());
    }

    pub fn u32(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.0[off..off + 4].try_into().unwrap())
    }

    pub fn set_u32(&mut self, off: usize, v: u32) {
        self.0[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn encode_hola(version: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::HOLA);
        m.set_u32(4, version);
        m
    }

    pub fn encode_crear(w: u32, h: u32, titulo_off: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::CREAR);
        m.set_u32(4, w);
        m.set_u32(8, h);
        m.set_u32(12, titulo_off);
        m
    }

    pub fn encode_ventana(id: u32, shm: u32, w: u32, h: u32, fmt: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::VENTANA);
        m.set_u32(4, id);
        m.set_u32(8, shm);
        m.set_u32(12, w);
        m.set_u32(16, h);
        m.set_u32(20, fmt);
        m
    }

    pub fn encode_tecla(key: u32, mods: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::TECLA);
        m.set_u32(4, key);
        m.set_u32(8, mods);
        m
    }

    pub fn encode_danio(x: u32, y: u32, w: u32, h: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::DANIO);
        m.set_u32(4, x);
        m.set_u32(8, y);
        m.set_u32(12, w);
        m.set_u32(16, h);
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_vectors() -> Vec<[u8; MSG_LEN]> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/escritorio/proto-vectores.bin");
        let raw = std::fs::read(&path).unwrap_or_default();
        assert!(
            !raw.is_empty() && raw.len() % MSG_LEN == 0,
            "missing or bad {}",
            path.display()
        );
        raw.chunks_exact(MSG_LEN)
            .map(|c| c.try_into().unwrap())
            .collect()
    }

    #[test]
    fn matches_golden_vectors() {
        let vecs = load_vectors();
        let hola = Msg::encode_hola(1);
        assert_eq!(vecs[0], *hola.as_bytes());
        let crear = Msg::encode_crear(640, 480, 16);
        assert_eq!(vecs[1], *crear.as_bytes());
        let ventana = Msg::encode_ventana(3, 9, 640, 480, 0);
        assert_eq!(vecs[2], *ventana.as_bytes());
        let tecla = Msg::encode_tecla(b'a' as u32, 2);
        assert_eq!(vecs[3], *tecla.as_bytes());
        let danio = Msg::encode_danio(10, 20, 100, 50);
        assert_eq!(vecs[4], *danio.as_bytes());
    }
}

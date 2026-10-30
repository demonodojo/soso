//! Mensajes de 32 bytes en little-endian (mismo layout que `proto.sal`).

pub mod gfram;
pub mod gpres;
pub mod gscen;
pub mod guprc;
pub mod nvdisp;
pub mod scanout;

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
    pub const TEMA: u32 = 23;
    pub const ESCALA: u32 = 24;
}

pub mod escala {
    pub const NIVELES: u32 = 5;
}

pub mod comp {
    pub const PORT: u16 = 7432;
    pub const ESCENA: u32 = 30;
    pub const FRAME: u32 = 31;
    pub const CERRAR: u32 = 32;
    pub const BIND: u32 = 33;
    pub const GEOM: u32 = 34;
    pub const DMAP: u32 = 35;
    pub const COMMIT: u32 = 36;

    pub const GEOM_HIDDEN: u32 = 1;
    pub const GEOM_OPAQUE: u32 = 2;
    pub const COMMIT_CLEAR_RT: u32 = 1;
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

    pub fn encode_ventana(id: u32, shm: u32, w: u32, h: u32, fmt: u32, tema: u32, escala: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::VENTANA);
        m.set_u32(4, id);
        m.set_u32(8, shm);
        m.set_u32(12, w);
        m.set_u32(16, h);
        m.set_u32(20, fmt);
        m.set_u32(24, tema);
        m.set_u32(28, escala);
        m
    }

    pub fn encode_tema(tema: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::TEMA);
        m.set_u32(4, tema);
        m
    }

    pub fn encode_escala(nivel: u32, gw: u32, gh: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(op::ESCALA);
        m.set_u32(4, nivel);
        m.set_u32(8, gw);
        m.set_u32(12, gh);
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

    pub fn encode_comp_escena(
        shm_id: u32,
        w: u32,
        h: u32,
        stride: u32,
        bpp: u32,
        fmt: u32,
        gpu_ok: u32,
    ) -> Self {
        let mut m = Self::zero();
        m.set_op(comp::ESCENA);
        m.set_u32(4, shm_id);
        m.set_u32(8, w);
        m.set_u32(12, h);
        m.set_u32(16, stride);
        m.set_u32(20, bpp);
        m.set_u32(24, fmt);
        m.set_u32(28, gpu_ok);
        m
    }

    pub fn encode_comp_bind(id: u32, shm_id: u32, w: u32, h: u32, stride: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(comp::BIND);
        m.set_u32(4, id);
        m.set_u32(8, shm_id);
        m.set_u32(12, w);
        m.set_u32(16, h);
        m.set_u32(20, stride);
        m
    }

    pub fn encode_comp_geom(id: u32, x: u32, y: u32, w: u32, h: u32, flags: u32, z: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(comp::GEOM);
        m.set_u32(4, id);
        m.set_u32(8, x);
        m.set_u32(12, y);
        m.set_u32(16, w);
        m.set_u32(20, h);
        m.set_u32(24, flags);
        m.set_u32(28, z);
        m
    }

    pub fn encode_comp_dmap(id: u32, x: u32, y: u32, w: u32, h: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(comp::DMAP);
        m.set_u32(4, id);
        m.set_u32(8, x);
        m.set_u32(12, y);
        m.set_u32(16, w);
        m.set_u32(20, h);
        m
    }

    pub fn encode_comp_commit(x: u32, y: u32, w: u32, h: u32, flags: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(comp::COMMIT);
        m.set_u32(4, x);
        m.set_u32(8, y);
        m.set_u32(12, w);
        m.set_u32(16, h);
        m.set_u32(20, flags);
        m
    }

    pub fn encode_comp_frame(x: u32, y: u32, w: u32, h: u32) -> Self {
        let mut m = Self::zero();
        m.set_op(comp::FRAME);
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
        let ventana = Msg::encode_ventana(3, 9, 640, 480, 0, 0, 0);
        assert_eq!(vecs[2], *ventana.as_bytes());
        let tecla = Msg::encode_tecla(b'a' as u32, 2);
        assert_eq!(vecs[3], *tecla.as_bytes());
        let danio = Msg::encode_danio(10, 20, 100, 50);
        assert_eq!(vecs[4], *danio.as_bytes());
    }

    #[test]
    fn comp_escena_y_frame() {
        let escena = Msg::encode_comp_escena(42, 1280, 800, 5120, 4, 1, 1);
        assert_eq!(escena.op(), comp::ESCENA);
        assert_eq!(escena.u32(4), 42);
        assert_eq!(escena.u32(8), 1280);
        assert_eq!(escena.u32(16), 5120);
        assert_eq!(escena.u32(20), 4);
        assert_eq!(escena.u32(24), 1);
        assert_eq!(escena.u32(28), 1);

        let frame = Msg::encode_comp_frame(10, 20, 100, 50);
        assert_eq!(frame.op(), comp::FRAME);
        assert_eq!(frame.u32(4), 10);
        assert_eq!(frame.u32(8), 20);
        assert_eq!(frame.u32(12), 100);
        assert_eq!(frame.u32(16), 50);
        assert_eq!(comp::PORT, 7432);

        let bind = Msg::encode_comp_bind(1, 2, 640, 480, 2560);
        assert_eq!(bind.op(), comp::BIND);
        assert_eq!(bind.u32(20), 2560);

        let geom = Msg::encode_comp_geom(1, 10, 20, 100, 50, 0, 5);
        assert_eq!(geom.u32(28), 5);

        let commit = Msg::encode_comp_commit(0, 0, 640, 480, comp::COMMIT_CLEAR_RT);
        assert_eq!(commit.op(), comp::COMMIT);
        assert_eq!(commit.u32(20), comp::COMMIT_CLEAR_RT);
    }

    #[test]
    fn tema_escala_y_ventana_offsets() {
        let v = Msg::encode_ventana(1, 2, 100, 200, 0, 1, 2);
        assert_eq!(v.u32(24), 1);
        assert_eq!(v.u32(28), 2);
        let t = Msg::encode_tema(0);
        assert_eq!(t.op(), op::TEMA);
        assert_eq!(t.u32(4), 0);
        let e = Msg::encode_escala(2, 12, 24);
        assert_eq!(e.op(), op::ESCALA);
        assert_eq!(e.u32(4), 2);
        assert_eq!(e.u32(8), 12);
        assert_eq!(e.u32(12), 24);
        assert_eq!(escala::NIVELES, 5);
    }
}

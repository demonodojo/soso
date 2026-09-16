//! Ring de bytes con **cursor monotónico**.
//!
//! El ring de consola ya existía, pero su única medida era `len()`, que se
//! satura al llenarse: a partir de ahí «creció» deja de ser detectable y el
//! volcado periódico se queda quieto aunque sigan entrando líneas. El cursor
//! `escritos` no se satura nunca, así que un lector puede saber exactamente
//! cuánto le falta —y cuánto se ha perdido por sobrescritura— sin depender de
//! la longitud.

/// Ring de capacidad fija que sobrescribe lo más antiguo.
pub struct Ring<const CAP: usize> {
    data: [u8; CAP],
    /// Siguiente índice de escritura (módulo CAP).
    pos: usize,
    /// Bytes válidos en el ring (como máximo CAP).
    len: usize,
    /// Bytes totales anexados desde el arranque. No se satura.
    escritos: u64,
}

/// Lo que un lector obtiene al avanzar su cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lectura {
    /// Bytes copiados a la salida.
    pub copiados: usize,
    /// Bytes que el ring sobrescribió antes de que el lector llegara a ellos.
    pub perdidos: u64,
    /// Cursor tras esta lectura.
    pub cursor: u64,
    /// Bytes que siguen pendientes en el ring después de esta lectura.
    pub pendientes: u64,
}

impl<const CAP: usize> Default for Ring<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> Ring<CAP> {
    pub const fn new() -> Self {
        Self { data: [0; CAP], pos: 0, len: 0, escritos: 0 }
    }

    pub fn append(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.data[self.pos] = b;
            self.pos = (self.pos + 1) % CAP;
            if self.len < CAP {
                self.len += 1;
            }
        }
        self.escritos = self.escritos.saturating_add(bytes.len() as u64);
    }

    fn start(&self) -> usize {
        (self.pos + CAP - self.len) % CAP
    }

    pub fn byte_len(&self) -> usize {
        self.len
    }

    /// Bytes anexados desde el arranque, se haya sobrescrito o no.
    pub fn escritos(&self) -> u64 {
        self.escritos
    }

    /// Cursor del byte más antiguo que todavía está en el ring.
    pub fn cursor_minimo(&self) -> u64 {
        self.escritos - self.len as u64
    }

    /// Copia desde `offset` contado sobre el contenido vivo (0 = más antiguo).
    pub fn copy_from(&self, offset: usize, out: &mut [u8]) -> usize {
        if offset >= self.len || out.is_empty() {
            return 0;
        }
        let n = out.len().min(self.len - offset);
        let mut idx = (self.start() + offset) % CAP;
        for slot in out.iter_mut().take(n) {
            *slot = self.data[idx];
            idx = (idx + 1) % CAP;
        }
        n
    }

    /// Copia a partir de un cursor absoluto.
    ///
    /// Si el ring ya sobrescribió lo que faltaba por leer, `perdidos` lo dice y
    /// la lectura se reanuda en el byte más antiguo disponible: se marca el
    /// hueco, no se inventan datos ni se duplican los que sí quedan.
    pub fn leer_desde(&self, cursor: u64, out: &mut [u8]) -> Lectura {
        let minimo = self.cursor_minimo();
        let (mut cursor, perdidos) = if cursor < minimo {
            (minimo, minimo - cursor)
        } else {
            (cursor.min(self.escritos), 0)
        };
        let offset = (cursor - minimo) as usize;
        let copiados = self.copy_from(offset, out);
        cursor += copiados as u64;
        Lectura {
            copiados,
            perdidos,
            cursor,
            pendientes: self.escritos - cursor,
        }
    }
}

//! Cabeceras y marcas de los ficheros de log. Sin `alloc`: el kernel las
//! formatea sobre un buffer de pila.

use core::fmt::Write;

pub struct EscritorSlice<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> EscritorSlice<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub fn len(&self) -> usize {
        self.pos
    }
    pub fn is_empty(&self) -> bool {
        self.pos == 0
    }
}

impl Write for EscritorSlice<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len().saturating_sub(self.pos));
        self.buf[self.pos..self.pos + n].copy_from_slice(&s.as_bytes()[..n]);
        self.pos += n;
        Ok(())
    }
}

/// Identificación de un arranque. Se escribe una vez por flujo y arranque, para
/// poder separar en un fichero acumulado lo que pasó en cada uno.
#[derive(Clone, Copy, Debug)]
pub struct Cabecera<'a> {
    pub flujo: &'a str,
    /// Identificador del arranque, único dentro de la instalación.
    pub boot_id: u64,
    pub version: &'a str,
    pub uptime_ms: u64,
    /// Fecha real, sólo cuando el reloj es utilizable. Si no, se omite: una
    /// fecha inventada en un log es peor que no tener fecha.
    pub fecha: Option<&'a str>,
}

pub fn cabecera(out: &mut [u8], c: &Cabecera<'_>) -> usize {
    let mut w = EscritorSlice::new(out);
    let _ = write!(
        w,
        "=== soso {} arranque {:016x} flujo {} monotónico {}ms",
        c.version, c.boot_id, c.flujo, c.uptime_ms
    );
    if let Some(f) = c.fecha {
        let _ = write!(w, " fecha {f}");
    }
    let _ = write!(w, " ===\n");
    w.len()
}

/// Marca explícita de pérdida. Sin ella, un ring que se llenó durante un
/// arranque ruidoso deja un salto invisible en el fichero.
pub fn marca_perdida(out: &mut [u8], bytes: u64) -> usize {
    let mut w = EscritorSlice::new(out);
    let _ = write!(w, "=== se perdieron {bytes} bytes por sobrescritura del ring ===\n");
    w.len()
}

/// Marca de que el propio escritor no pudo persistir. Va a consola y al ring,
/// nunca al fichero que acaba de fallar.
pub fn marca_fallo(out: &mut [u8], flujo: &str, fallos: u32) -> usize {
    let mut w = EscritorSlice::new(out);
    let _ = write!(w, "logfs: {flujo}: escritura fallida ({fallos} seguidos)\n");
    w.len()
}

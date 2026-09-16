//! Política de rotación de los ficheros de log en sosofs.
//!
//! Append por lotes sobre un fichero activo acotado; al llenarse se corre el
//! historial un puesto y se empieza de cero. No se reescribe un historial
//! creciente cada dos segundos: eso, con CoW, duplicaría el fichero entero en
//! cada volcado.

/// Tamaño máximo del fichero activo de cada flujo.
pub const MAX_ACTIVO: u64 = 1024 * 1024;
/// Ficheros rotados que se conservan por flujo (`.1`, `.2`, `.3`).
pub const ROTACIONES: u8 = 3;

/// Techo por flujo: activo + rotados. Tres flujos ≈ 12 MiB, que es la reserva
/// que `soso-update-core` descuenta al comprobar capacidad antes de una OTA.
pub const TECHO_POR_FLUJO: u64 = MAX_ACTIVO * (ROTACIONES as u64 + 1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accion {
    /// Cabe: anexar al fichero activo.
    Anexar,
    /// No cabe: rotar primero y anexar al activo nuevo.
    RotarYAnexar,
}

/// Un lote que por sí solo supera el máximo no provoca una rotación infinita:
/// se rota una vez y se escribe entero. Perder mensajes por ser grandes sería
/// peor que un fichero algo mayor que el tope.
pub fn decidir(activo_bytes: u64, lote: usize) -> Accion {
    if activo_bytes == 0 || activo_bytes + lote as u64 <= MAX_ACTIVO {
        Accion::Anexar
    } else {
        Accion::RotarYAnexar
    }
}

/// Un paso de la rotación. `0` es el fichero activo; `1..=ROTACIONES` los
/// históricos. El orden importa: se va del más viejo al más nuevo para no
/// pisar un fichero que todavía hace falta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paso {
    Borrar(u8),
    Renombrar { de: u8, a: u8 },
}

/// Pasos completos de una rotación, en orden de ejecución.
pub fn pasos() -> [Paso; ROTACIONES as usize + 1] {
    let mut out = [Paso::Borrar(ROTACIONES); ROTACIONES as usize + 1];
    let mut i = 1;
    let mut n = ROTACIONES;
    while n >= 1 {
        out[i] = Paso::Renombrar { de: n - 1, a: n };
        i += 1;
        if n == 1 {
            break;
        }
        n -= 1;
    }
    out
}

/// Sufijo de un índice: `""` para el activo, `".2"` para el histórico 2.
pub fn sufijo(indice: u8, out: &mut [u8; 4]) -> usize {
    if indice == 0 {
        return 0;
    }
    out[0] = b'.';
    out[1] = b'0' + indice;
    2
}

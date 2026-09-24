//! Runner de pruebas sin libtest, para poder ejecutarlas **dentro de soso** (T49).
//!
//! soso no tiene `libtest`, así que las pruebas que acreditan el circuito no
//! pueden ser `#[test]`. Aquí están los tipos que permiten escribirlas una vez
//! y ejecutarlas en los dos sitios: el host las envuelve en `#[test]`, el guest
//! las ejecuta con `soso-improve pruebas`.
//!
//! Lo que distingue a este runner de contar aciertos es el estado
//! [`Estado::Pendiente`]: una capacidad que **todavía no existe** —compilar un
//! programa, resolver un repo— no es un acierto ni un fallo. Contarla como
//! acierto infla el resultado; excluirla del total lo infla igual, porque
//! cambia el denominador. Se cuenta, se informa, y se dice qué ficha la traerá.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ESQUEMA;

/// Cómo acabó una prueba.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Estado {
    Paso,
    /// La prueba se ejecutó y falló. `detalle` dice qué aserción.
    Fallo(String),
    /// La capacidad no existe todavía; el texto es la ficha que la traerá.
    /// **Cuenta en el total** y no cuenta como acierto.
    Pendiente(String),
    /// No se pudo ejecutar: el entorno falló antes de poder juzgar nada.
    Error(String),
}

impl Estado {
    pub fn nombre(&self) -> &'static str {
        match self {
            Estado::Paso => "paso",
            Estado::Fallo(_) => "fallo",
            Estado::Pendiente(_) => "pendiente",
            Estado::Error(_) => "error",
        }
    }

    pub fn detalle(&self) -> &str {
        match self {
            Estado::Paso => "",
            Estado::Fallo(d) | Estado::Pendiente(d) | Estado::Error(d) => d,
        }
    }

    pub fn paso(&self) -> bool {
        matches!(self, Estado::Paso)
    }
}

/// Resultado de una prueba concreta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prueba {
    pub id: String,
    /// Qué ficha o área acredita: `T46`, `T47`, `banco`…
    pub target: String,
    pub estado: Estado,
    pub ms: u64,
}

impl Prueba {
    pub fn nueva(id: &str, target: &str, estado: Estado, ms: u64) -> Self {
        Prueba {
            id: id.to_string(),
            target: target.to_string(),
            estado,
            ms,
        }
    }
}

/// Cuentas de una tanda. `total` incluye las pendientes a propósito.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resumen {
    pub total: usize,
    pub pasaron: usize,
    pub fallaron: usize,
    pub pendientes: usize,
    pub errores: usize,
}

impl Resumen {
    /// Sólo hay éxito si **nada** falló ni dio error. Las pendientes no
    /// impiden el éxito —no son un defecto— pero se informan siempre.
    pub fn ok(&self) -> bool {
        self.fallaron == 0 && self.errores == 0
    }
}

/// El informe completo de una ejecución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Informe {
    pub schema_version: u32,
    /// `host` o `guest`: el mismo informe se compara entre los dos.
    pub plataforma: String,
    pub pruebas: Vec<Prueba>,
    pub resumen: Resumen,
}

impl Informe {
    pub fn nuevo(plataforma: &str, pruebas: Vec<Prueba>) -> Self {
        let mut r = Resumen {
            total: pruebas.len(),
            ..Default::default()
        };
        for p in &pruebas {
            match p.estado {
                Estado::Paso => r.pasaron += 1,
                Estado::Fallo(_) => r.fallaron += 1,
                Estado::Pendiente(_) => r.pendientes += 1,
                Estado::Error(_) => r.errores += 1,
            }
        }
        Informe {
            schema_version: ESQUEMA,
            plataforma: plataforma.to_string(),
            pruebas,
            resumen: r,
        }
    }

    /// JSON estable, escrito a mano porque el crate es `no_std`.
    pub fn json(&self) -> String {
        let mut s = format!(
            "{{\"schema_version\":{},\"plataforma\":\"{}\",\"resumen\":{{\"total\":{},\"pasaron\":{},\"fallaron\":{},\"pendientes\":{},\"errores\":{}}},\"pruebas\":[",
            self.schema_version,
            escapar(&self.plataforma),
            self.resumen.total,
            self.resumen.pasaron,
            self.resumen.fallaron,
            self.resumen.pendientes,
            self.resumen.errores
        );
        for (i, p) in self.pruebas.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"id\":\"{}\",\"target\":\"{}\",\"estado\":\"{}\",\"ms\":{},\"detalle\":\"{}\"}}",
                escapar(&p.id),
                escapar(&p.target),
                p.estado.nombre(),
                p.ms,
                escapar(p.estado.detalle())
            ));
        }
        s.push_str("]}");
        s
    }

    /// Línea por prueba, para leerlo por la consola serie sin herramientas.
    pub fn texto(&self) -> String {
        let mut s = String::new();
        for p in &self.pruebas {
            s.push_str(&format!(
                "{:9} {:6} {:>6} ms  {}",
                p.estado.nombre(),
                p.target,
                p.ms,
                p.id
            ));
            if !p.estado.detalle().is_empty() {
                s.push_str(&format!("  ({})", p.estado.detalle()));
            }
            s.push('\n');
        }
        s.push_str(&format!(
            "pruebas: {} total, {} pasaron, {} fallaron, {} pendientes, {} errores\n",
            self.resumen.total,
            self.resumen.pasaron,
            self.resumen.fallaron,
            self.resumen.pendientes,
            self.resumen.errores
        ));
        s
    }
}

fn escapar(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pruebas() -> Vec<Prueba> {
        alloc::vec![
            Prueba::nueva("captura/reconstruir", "T01", Estado::Paso, 12),
            Prueba::nueva("banco/validar", "T02", Estado::Paso, 3),
            Prueba::nueva(
                "verificar/programa",
                "T40",
                Estado::Pendiente(String::from("necesita compilador: T40")),
                0
            ),
            Prueba::nueva(
                "verificar/repo",
                "T41",
                Estado::Pendiente(String::from("necesita cargo: T41")),
                0
            ),
        ]
    }

    /// Lo que esta ficha decide: una capacidad pendiente **cuenta en el
    /// total** y no cuenta como acierto. Excluirla inflaría el porcentaje
    /// igual que contarla como buena.
    #[test]
    fn las_pendientes_cuentan_en_el_total_y_no_como_acierto() {
        let i = Informe::nuevo("guest", pruebas());
        assert_eq!(i.resumen.total, 4);
        assert_eq!(i.resumen.pasaron, 2);
        assert_eq!(i.resumen.pendientes, 2);
        assert_eq!(i.resumen.fallaron, 0);
        // Y aun así la tanda es correcta: no hay defecto, hay trabajo futuro.
        assert!(i.resumen.ok());
    }

    #[test]
    fn un_fallo_hunde_la_tanda() {
        let mut p = pruebas();
        p.push(Prueba::nueva(
            "protocolo/Q01",
            "T12",
            Estado::Fallo(String::from("2/4 aserciones")),
            7,
        ));
        let i = Informe::nuevo("guest", p);
        assert!(!i.resumen.ok());
        assert_eq!(i.resumen.fallaron, 1);
        assert_eq!(i.resumen.total, 5);
    }

    /// Un error del entorno no es un fallo de la prueba, y se cuenta aparte.
    #[test]
    fn un_error_no_es_un_fallo() {
        let i = Informe::nuevo(
            "guest",
            alloc::vec![Prueba::nueva(
                "durable/reinicio",
                "T46",
                Estado::Error(String::from("no pude leer /var")),
                1
            )],
        );
        assert_eq!(i.resumen.errores, 1);
        assert_eq!(i.resumen.fallaron, 0);
        assert!(!i.resumen.ok());
    }

    #[test]
    fn el_json_lleva_cuentas_y_detalle() {
        let i = Informe::nuevo("guest", pruebas());
        let j = i.json();
        assert!(j.contains("\"plataforma\":\"guest\""), "{j}");
        assert!(j.contains("\"total\":4"), "{j}");
        assert!(j.contains("\"pendientes\":2"), "{j}");
        assert!(j.contains("\"estado\":\"pendiente\""), "{j}");
        assert!(j.contains("T40"), "{j}");
    }

    #[test]
    fn el_texto_se_lee_por_la_consola() {
        let t = Informe::nuevo("guest", pruebas()).texto();
        assert!(t.contains("captura/reconstruir"));
        assert!(t.contains("pendiente"));
        assert!(t.contains("4 total, 2 pasaron"));
    }
}

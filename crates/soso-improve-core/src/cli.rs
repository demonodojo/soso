//! Órdenes, capacidades y códigos de salida compartidos por los dos frontends (T45).
//!
//! Hasta ahora `tools/soso-improve` (host) y `user/soso-improve` (guest)
//! tenían **sintaxis distinta** —`--clave valor` frente a posicional— y, lo que
//! es peor, criterios distintos para el éxito: el guest imprimía el fallo de
//! una verificación y devolvía `Ok(())`, es decir, exit code 0. Un arnés que
//! mire el código de salida daba por buena una reconstrucción rota.
//!
//! Aquí viven las tres cosas que tienen que ser iguales en los dos lados:
//!
//! - qué **órdenes** existen y cómo se nombran sus argumentos ([`Orden`]);
//! - qué **capacidades** están de verdad implementadas en cada frontend
//!   ([`Capacidades`]), para poder rechazar antes del primer efecto;
//! - qué **código de salida** corresponde a cada desenlace ([`Codigo`]).

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::{format, vec};

use crate::{Error, Resultado, ESQUEMA};

/// Código de salida del proceso. Es la única señal que un arnés puede leer sin
/// interpretar texto, así que cada valor significa una cosa y solo una.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codigo {
    /// Todo bien.
    Exito = 0,
    /// No se pudo usar: uso incorrecto, entorno roto o capacidad ausente.
    Error = 1,
    /// La medida se hizo y **falló**: reconstrucción con problemas, banco
    /// inválido, aserciones de protocolo que no pasan.
    Verificacion = 2,
    /// El checkout cambió mientras se capturaba.
    Inestable = 3,
    /// Alguna suite falló.
    Suite = 4,
}

impl Codigo {
    pub fn como_u8(self) -> u8 {
        self as u8
    }

    pub fn como_i32(self) -> i32 {
        self as i32
    }

    /// Clasificación de un error del core. `Inestable` tiene código propio
    /// porque no es un fallo de la medida: es que no se pudo medir.
    pub fn de_error(e: &Error) -> Codigo {
        match e {
            Error::Inestable(_) => Codigo::Inestable,
            _ => Codigo::Error,
        }
    }
}

/// Qué sabe hacer un frontend. Se publica para poder decir «esto aquí no» con
/// la ficha que lo traerá, en vez de fingir que se ejecutó.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capacidad {
    Capturar,
    Reconstruir,
    Suites,
    BancoListar,
    BancoValidar,
    BancoSellar,
    VerificarProtocolo,
    /// Necesita un compilador de verdad (T40).
    VerificarPrograma,
    /// Necesita cargo dentro de soso (T41).
    VerificarRepo,
    /// Perfil y comparación del modelo (T03).
    Modelo,
    /// Eco controlado entre procesos: comprueba reloj y transporte (T48).
    Eco,
    /// Campaña go/no-go contra un endpoint real (T14).
    Evaluar,
    /// Autoprueba del adaptador de procesos (T47).
    Procesos,
    /// Runner de pruebas dentro de soso (T49).
    Pruebas,
    /// Aplicar un paquete de cambios sin Git (T50).
    Delta,
    /// Crear el estado durable de una tarea (T23).
    TareaPreparar,
    /// Lanzar al candidato sobre la tarea. Necesita el ejecutor (T25).
    TareaEjecutar,
    /// Juzgar un candidato y, sólo entonces, aceptarlo. Necesita T26.
    TareaValidar,
    /// Decir por dónde se quedó una ejecución (T23); repetir sin duplicar
    /// efectos es T27.
    TareaReanudar,
    /// Volcar el estado de una ejecución (T23).
    TareaInforme,
    /// Copia de tarea: reconstruir la base y sembrar el enunciado (T24).
    TareaCopiar,
    /// Exportar lo que cambió en la copia como paquete aplicable (T24).
    TareaExportar,
    /// Decir qué parches del bootstrap de libstd faltan en un vendor, **sin
    /// tocarlo** (T39). Existe porque el vendor se quedaba a medias en
    /// silencio: un `sed` que falla, o un script que aborta a la mitad, dejan
    /// exactamente ese estado y no hay forma de verlo salvo abriendo ficheros.
    RecetaComprobar,
}

impl Capacidad {
    pub fn nombre(self) -> &'static str {
        match self {
            Capacidad::Capturar => "capturar",
            Capacidad::Reconstruir => "reconstruir",
            Capacidad::Suites => "suites",
            Capacidad::BancoListar => "banco listar",
            Capacidad::BancoValidar => "banco validar",
            Capacidad::BancoSellar => "banco sellar",
            Capacidad::VerificarProtocolo => "verificar protocolo",
            Capacidad::VerificarPrograma => "verificar programa",
            Capacidad::VerificarRepo => "verificar repo",
            Capacidad::Modelo => "modelo",
            Capacidad::Eco => "eco",
            Capacidad::Evaluar => "evaluar",
            Capacidad::Procesos => "procesos",
            Capacidad::Pruebas => "pruebas",
            Capacidad::Delta => "delta",
            Capacidad::TareaPreparar => "tarea preparar",
            Capacidad::TareaEjecutar => "tarea ejecutar",
            Capacidad::TareaValidar => "tarea validar",
            Capacidad::TareaReanudar => "tarea reanudar",
            Capacidad::TareaInforme => "tarea informe",
            Capacidad::TareaCopiar => "tarea copiar",
            Capacidad::TareaExportar => "tarea exportar",
            Capacidad::RecetaComprobar => "receta comprobar",
        }
    }

    /// Ficha que traerá la capacidad, cuando se sabe cuál es. Sirve para que el
    /// mensaje de «capacidad ausente» diga algo accionable.
    pub fn ficha_pendiente(self) -> Option<&'static str> {
        match self {
            Capacidad::VerificarPrograma => Some("T40"),
            Capacidad::VerificarRepo => Some("T41"),
            Capacidad::Suites => Some("T47"),
            Capacidad::TareaEjecutar => Some("T25"),
            Capacidad::TareaValidar => Some("T26"),
            _ => None,
        }
    }
}

/// Las capacidades que un frontend declara tener.
#[derive(Debug, Clone)]
pub struct Capacidades {
    /// Cómo se llama esta plataforma en los informes: `host` o `guest`.
    pub plataforma: &'static str,
    presentes: Vec<Capacidad>,
}

impl Capacidades {
    pub fn nueva(plataforma: &'static str, presentes: &[Capacidad]) -> Self {
        let mut presentes = presentes.to_vec();
        presentes.sort();
        presentes.dedup();
        Capacidades {
            plataforma,
            presentes,
        }
    }

    pub fn tiene(&self, c: Capacidad) -> bool {
        self.presentes.binary_search(&c).is_ok()
    }

    pub fn lista(&self) -> &[Capacidad] {
        &self.presentes
    }

    /// Error de capacidad ausente, con la ficha que la traerá si se sabe.
    pub fn ausente(&self, c: Capacidad) -> Error {
        match c.ficha_pendiente() {
            Some(ficha) => Error::uso(format!(
                "capacidad ausente en {}: «{}» llega en {ficha}",
                self.plataforma,
                c.nombre()
            )),
            None => Error::uso(format!(
                "capacidad ausente en {}: «{}»",
                self.plataforma,
                c.nombre()
            )),
        }
    }

    /// Comprueba antes del primer efecto. C5: nada de empezar a escribir y
    /// descubrir a la mitad que la orden no se podía atender.
    pub fn exigir(&self, c: Capacidad) -> Resultado<()> {
        if self.tiene(c) {
            Ok(())
        } else {
            Err(self.ausente(c))
        }
    }
}

/// Una orden ya interpretada: nombre canónico, subcomando y argumentos con
/// nombre. Es serializable a propósito —T47 tendrá que pasarla a un proceso— y
/// no depende de cómo la escribiera el usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orden {
    pub nombre: String,
    pub sub: Option<String>,
    valores: BTreeMap<String, String>,
    banderas: Vec<String>,
}

impl Orden {
    pub fn nueva(nombre: &str, sub: Option<&str>) -> Self {
        Orden {
            nombre: nombre.to_string(),
            sub: sub.map(|s| s.to_string()),
            valores: BTreeMap::new(),
            banderas: Vec::new(),
        }
    }

    pub fn con(mut self, clave: &str, valor: &str) -> Self {
        self.valores.insert(clave.to_string(), valor.to_string());
        self
    }

    pub fn con_bandera(mut self, clave: &str) -> Self {
        if !self.banderas.iter().any(|b| b == clave) {
            self.banderas.push(clave.to_string());
        }
        self
    }

    pub fn uno(&self, clave: &str) -> Option<&str> {
        self.valores.get(clave).map(|s| s.as_str())
    }

    pub fn exigido(&self, clave: &str) -> Resultado<&str> {
        self.uno(clave)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::uso(format!("falta --{clave} en «{}»", self.etiqueta())))
    }

    pub fn bandera(&self, clave: &str) -> bool {
        self.banderas.iter().any(|b| b == clave)
    }

    /// `capturar` o `banco validar`: cómo nombrar la orden en un mensaje.
    pub fn etiqueta(&self) -> String {
        match &self.sub {
            Some(s) => format!("{} {s}", self.nombre),
            None => self.nombre.clone(),
        }
    }

    /// Qué capacidad hace falta para atenderla.
    pub fn capacidad(&self) -> Resultado<Capacidad> {
        let sub = self.sub.as_deref().unwrap_or("");
        match (self.nombre.as_str(), sub) {
            ("capturar", _) => Ok(Capacidad::Capturar),
            ("reconstruir", _) => Ok(Capacidad::Reconstruir),
            ("suites", _) => Ok(Capacidad::Suites),
            ("modelo", _) => Ok(Capacidad::Modelo),
            ("eco", _) => Ok(Capacidad::Eco),
            ("evaluar", _) => Ok(Capacidad::Evaluar),
            ("procesos", _) => Ok(Capacidad::Procesos),
            ("pruebas", _) => Ok(Capacidad::Pruebas),
            ("delta", _) => Ok(Capacidad::Delta),
            ("tarea", "preparar") => Ok(Capacidad::TareaPreparar),
            ("tarea", "ejecutar") => Ok(Capacidad::TareaEjecutar),
            ("tarea", "validar") => Ok(Capacidad::TareaValidar),
            ("tarea", "reanudar") => Ok(Capacidad::TareaReanudar),
            ("tarea", "informe") => Ok(Capacidad::TareaInforme),
            ("tarea", "copiar") => Ok(Capacidad::TareaCopiar),
            ("tarea", "exportar") => Ok(Capacidad::TareaExportar),
            ("tarea", otro) => Err(Error::uso(format!(
                "tarea necesita preparar|copiar|exportar|ejecutar|validar|reanudar|informe, no «{otro}»"
            ))),
            ("eco-servidor", _) => Ok(Capacidad::Eco),
            ("banco", "listar") => Ok(Capacidad::BancoListar),
            ("banco", "validar") => Ok(Capacidad::BancoValidar),
            ("banco", "sellar") => Ok(Capacidad::BancoSellar),
            ("banco", otro) => Err(Error::uso(format!(
                "banco necesita listar|validar|sellar, no «{otro}»"
            ))),
            ("receta", "comprobar") => Ok(Capacidad::RecetaComprobar),
            ("receta", otro) => Err(Error::uso(format!(
                "receta necesita comprobar, no «{otro}»"
            ))),
            ("verificar", "protocolo") => Ok(Capacidad::VerificarProtocolo),
            ("verificar", "programa") => Ok(Capacidad::VerificarPrograma),
            ("verificar", "repo") => Ok(Capacidad::VerificarRepo),
            ("verificar", otro) => Err(Error::uso(format!(
                "verificar necesita programa|protocolo|repo, no «{otro}»"
            ))),
            (otro, _) => Err(Error::uso(format!("orden desconocida: {otro}"))),
        }
    }

    /// Interpreta `argv` (sin el nombre del programa).
    ///
    /// Acepta las dos sintaxis que había: `--clave valor` (host) y la
    /// **posicional** del guest, que se mantiene como alias para no romper
    /// nada escrito contra ella. Los alias posicionales están declarados, no
    /// adivinados: cada orden dice qué significa cada posición.
    pub fn parsear(argv: &[&str]) -> Resultado<Orden> {
        let Some((nombre, resto)) = argv.split_first() else {
            return Err(Error::uso("falta la orden"));
        };
        let nombre = *nombre;
        let (sub, resto) = match nombre {
            "banco" | "verificar" | "modelo" | "tarea" | "receta" => match resto.split_first() {
                Some((s, r)) if !s.starts_with("--") => (Some(*s), r),
                _ => (None, resto),
            },
            _ => (None, resto),
        };

        let mut orden = Orden::nueva(nombre, sub);
        let posicionales: Vec<&str> = resto.iter().copied().filter(|a| !a.is_empty()).collect();

        // Sintaxis con nombre: en cuanto aparece un `--`, manda ella entera.
        if posicionales.iter().any(|a| a.starts_with("--")) {
            let mut i = 0;
            while i < posicionales.len() {
                let arg = posicionales[i];
                let Some(clave) = arg.strip_prefix("--") else {
                    return Err(Error::uso(format!("argumento suelto: {arg}")));
                };
                match posicionales.get(i + 1) {
                    Some(v) if !v.starts_with("--") => {
                        orden = orden.con(clave, v);
                        i += 2;
                    }
                    _ => {
                        orden = orden.con_bandera(clave);
                        i += 1;
                    }
                }
            }
            orden.validar_sub()?;
            return Ok(orden);
        }

        // Alias posicionales, uno por orden y documentados.
        let posiciones: &[&str] = match (nombre, sub) {
            ("capturar", _) => &["repo", "out"],
            ("eco", _) | ("eco-servidor", _) => &["puerto"],
            ("pruebas", _) => &["banco"],
            ("reconstruir", _) => &["captura", "destino"],
            ("banco", _) => &["banco"],
            ("tarea", Some("preparar")) => &["estado", "spec"],
            ("tarea", Some("copiar")) => &["estado", "run"],
            ("tarea", Some("exportar")) => &["estado", "run"],
            ("tarea", _) => &["estado", "run"],
            ("receta", Some("comprobar")) => &["vendor", "plantillas"],
            ("verificar", Some("protocolo")) => &["banco", "caso", "respuesta"],
            ("verificar", Some("programa")) => &["caso", "candidato"],
            ("verificar", Some("repo")) => &["caso", "arbol"],
            // `protocolo <dir> <caso> <respuesta>` era el nombre de primer
            // nivel en el guest; se conserva como alias de `verificar protocolo`.
            ("protocolo", _) => &["banco", "caso", "respuesta"],
            ("programa", _) => &["caso", "candidato"],
            ("repo", _) => &["caso", "arbol"],
            _ => &[],
        };
        if posicionales.len() > posiciones.len() {
            return Err(Error::uso(format!(
                "«{}» admite {} argumento(s) posicional(es), se dieron {}",
                orden.etiqueta(),
                posiciones.len(),
                posicionales.len()
            )));
        }
        for (clave, valor) in posiciones.iter().zip(posicionales.iter()) {
            orden = orden.con(clave, valor);
        }
        // Los alias de primer nivel se normalizan al nombre canónico para que
        // el despacho y los informes no tengan que conocerlos.
        if matches!(nombre, "protocolo" | "programa" | "repo") {
            let sub = nombre;
            let mut canonica = Orden::nueva("verificar", Some(sub));
            canonica.valores = orden.valores;
            canonica.banderas = orden.banderas;
            orden = canonica;
        }
        orden.validar_sub()?;
        Ok(orden)
    }

    fn validar_sub(&self) -> Resultado<()> {
        // `capacidad()` ya sabe qué combinaciones existen; usarla aquí evita
        // dos listas que se desincronizan.
        self.capacidad().map(|_| ())
    }
}

/// Lo que un frontend devuelve: el desenlace, en una forma que se puede
/// comparar entre host y guest sin leer texto libre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Informe {
    pub schema_version: u32,
    pub orden: String,
    pub plataforma: String,
    pub codigo: Codigo,
    /// Cuántas cosas fueron mal en una medida que **sí** se pudo hacer.
    pub problemas: usize,
    pub lineas: Vec<String>,
}

impl Informe {
    pub fn exito(orden: &Orden, plataforma: &str) -> Self {
        Informe {
            schema_version: ESQUEMA,
            orden: orden.etiqueta(),
            plataforma: plataforma.to_string(),
            codigo: Codigo::Exito,
            problemas: 0,
            lineas: Vec::new(),
        }
    }

    /// Una medida que se hizo y falló. **No** es lo mismo que un error: aquí
    /// el código es 2, y ése era justo el agujero del guest, que imprimía el
    /// fallo y salía con 0.
    pub fn verificacion_fallida(orden: &Orden, plataforma: &str, problemas: usize) -> Self {
        Informe {
            schema_version: ESQUEMA,
            orden: orden.etiqueta(),
            plataforma: plataforma.to_string(),
            codigo: Codigo::Verificacion,
            problemas,
            lineas: Vec::new(),
        }
    }

    pub fn de_error(orden: &Orden, plataforma: &str, e: &Error) -> Self {
        Informe {
            schema_version: ESQUEMA,
            orden: orden.etiqueta(),
            plataforma: plataforma.to_string(),
            codigo: Codigo::de_error(e),
            problemas: 0,
            lineas: vec![e.to_string()],
        }
    }

    pub fn con_linea(mut self, l: impl ToString) -> Self {
        self.lineas.push(l.to_string());
        self
    }

    /// Resultado de contar problemas: 0 → éxito, >0 → verificación fallida.
    pub fn segun_problemas(orden: &Orden, plataforma: &str, problemas: usize) -> Self {
        if problemas == 0 {
            Informe::exito(orden, plataforma)
        } else {
            Informe::verificacion_fallida(orden, plataforma, problemas)
        }
    }

    pub fn ok(&self) -> bool {
        self.codigo == Codigo::Exito
    }

    /// JSON estable, escrito a mano porque este crate es `no_std`.
    pub fn json(&self) -> String {
        let mut s = format!(
            "{{\"schema_version\":{},\"orden\":\"{}\",\"plataforma\":\"{}\",\"codigo\":{},\"problemas\":{},\"lineas\":[",
            self.schema_version,
            escapar(&self.orden),
            escapar(&self.plataforma),
            self.codigo.como_u8(),
            self.problemas
        );
        for (i, l) in self.lineas.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push('"');
            s.push_str(&escapar(l));
            s.push('"');
        }
        s.push_str("]}");
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
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Argv de una orden guardada en un archivo, una línea por argumento.
///
/// `split_whitespace` **no** conserva argv: una ruta con espacios se parte en
/// dos y el programa recibe otra cosa. Para órdenes complejas, el argv viaja
/// por archivo. Líneas vacías y `#` iniciales se ignoran.
pub fn argv_de_texto(texto: &str, max_args: usize) -> Resultado<Vec<String>> {
    let mut fuera = Vec::new();
    for linea in texto.lines() {
        let l = linea.strip_suffix('\r').unwrap_or(linea);
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if fuera.len() == max_args {
            return Err(Error::uso(format!(
                "el archivo de orden pasa de {max_args} argumentos"
            )));
        }
        fuera.push(l.to_string());
    }
    if fuera.is_empty() {
        return Err(Error::uso("el archivo de orden está vacío"));
    }
    Ok(fuera)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Capacidades {
        Capacidades::nueva(
            "host",
            &[
                Capacidad::Capturar,
                Capacidad::Reconstruir,
                Capacidad::Suites,
                Capacidad::BancoListar,
                Capacidad::BancoValidar,
                Capacidad::BancoSellar,
                Capacidad::VerificarProtocolo,
                Capacidad::VerificarPrograma,
                Capacidad::VerificarRepo,
                Capacidad::Modelo,
            ],
        )
    }

    fn guest() -> Capacidades {
        Capacidades::nueva(
            "guest",
            &[
                Capacidad::Capturar,
                Capacidad::Reconstruir,
                Capacidad::BancoListar,
                Capacidad::BancoValidar,
                Capacidad::VerificarProtocolo,
            ],
        )
    }

    /// El objetivo de la ficha: la misma orden escrita de las dos maneras que
    /// existían tiene que dar exactamente lo mismo.
    #[test]
    fn las_dos_sintaxis_coinciden() {
        let con_nombre = Orden::parsear(&["capturar", "--repo", ".", "--out", "/tmp/base"]).unwrap();
        let posicional = Orden::parsear(&["capturar", ".", "/tmp/base"]).unwrap();
        assert_eq!(con_nombre, posicional);
        assert_eq!(con_nombre.exigido("repo").unwrap(), ".");
        assert_eq!(con_nombre.exigido("out").unwrap(), "/tmp/base");
    }

    #[test]
    fn alias_de_primer_nivel_se_normaliza() {
        let alias = Orden::parsear(&["protocolo", "/banco", "Q01", "r.json"]).unwrap();
        let canonica =
            Orden::parsear(&["verificar", "protocolo", "/banco", "Q01", "r.json"]).unwrap();
        assert_eq!(alias, canonica);
        assert_eq!(alias.etiqueta(), "verificar protocolo");
        assert_eq!(alias.exigido("caso").unwrap(), "Q01");
    }

    #[test]
    fn orden_desconocida_y_subcomando_malo() {
        assert!(Orden::parsear(&["inventada"]).is_err());
        let e = Orden::parsear(&["banco", "inventado"]).unwrap_err();
        assert!(format!("{e}").contains("listar|validar|sellar"), "{e}");
    }

    #[test]
    fn sobran_posicionales() {
        let e = Orden::parsear(&["reconstruir", "a", "b", "c"]).unwrap_err();
        assert!(format!("{e}").contains("posicional"), "{e}");
    }

    #[test]
    fn falta_un_argumento_exigido() {
        let o = Orden::parsear(&["capturar", "--repo", "."]).unwrap();
        let e = o.exigido("out").unwrap_err();
        assert!(format!("{e}").contains("--out"), "{e}");
    }

    /// Una capacidad ausente se rechaza **antes** de cualquier efecto, y el
    /// mensaje dice qué ficha la traerá.
    #[test]
    fn capacidad_ausente_nombra_su_ficha() {
        let o = Orden::parsear(&["verificar", "programa", "--caso", "P01"]).unwrap();
        let cap = o.capacidad().unwrap();
        assert!(host().tiene(cap));
        let e = guest().exigir(cap).unwrap_err();
        let texto = format!("{e}");
        assert!(texto.contains("capacidad ausente"), "{texto}");
        assert!(texto.contains("guest"), "{texto}");
        assert!(texto.contains("T40"), "{texto}");
    }

    #[test]
    fn codigos_de_salida() {
        assert_eq!(Codigo::Exito.como_i32(), 0);
        assert_eq!(Codigo::Error.como_i32(), 1);
        assert_eq!(Codigo::Verificacion.como_i32(), 2);
        assert_eq!(Codigo::Inestable.como_i32(), 3);
        assert_eq!(Codigo::Suite.como_i32(), 4);
        assert_eq!(
            Codigo::de_error(&Error::Inestable("x".into())),
            Codigo::Inestable
        );
        assert_eq!(Codigo::de_error(&Error::uso("x")), Codigo::Error);
        assert_eq!(Codigo::de_error(&Error::formato("x")), Codigo::Error);
    }

    /// El agujero que cierra T45: una verificación que falla **no** puede
    /// salir con 0.
    #[test]
    fn verificacion_fallida_no_es_exito() {
        let o = Orden::parsear(&["reconstruir", "c", "d"]).unwrap();
        let bien = Informe::segun_problemas(&o, "guest", 0);
        assert!(bien.ok());
        assert_eq!(bien.codigo.como_i32(), 0);

        let mal = Informe::segun_problemas(&o, "guest", 3);
        assert!(!mal.ok());
        assert_eq!(mal.codigo.como_i32(), 2);
        assert_eq!(mal.problemas, 3);
    }

    #[test]
    fn informe_json_estable_y_escapado() {
        let o = Orden::parsear(&["banco", "validar", "--banco", "/b"]).unwrap();
        let i = Informe::verificacion_fallida(&o, "guest", 1).con_linea("dijo \"no\"\ny paró");
        let j = i.json();
        assert!(j.contains("\"schema_version\":1"), "{j}");
        assert!(j.contains("\"orden\":\"banco validar\""), "{j}");
        assert!(j.contains("\"codigo\":2"), "{j}");
        assert!(j.contains("\"problemas\":1"), "{j}");
        assert!(j.contains("\\\"no\\\""), "{j}");
        assert!(j.contains("\\n"), "{j}");
    }

    /// `split_whitespace` no conserva argv: una ruta con espacios se parte.
    #[test]
    fn argv_por_archivo_conserva_espacios() {
        let texto = "capturar\n--repo\n/con espacios/repo\n--out\n/tmp/base\n";
        let argv = argv_de_texto(texto, 16).unwrap();
        assert_eq!(argv[2], "/con espacios/repo");
        let refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        let o = Orden::parsear(&refs).unwrap();
        assert_eq!(o.exigido("repo").unwrap(), "/con espacios/repo");
    }

    #[test]
    fn argv_por_archivo_con_limites() {
        assert!(argv_de_texto("", 8).is_err());
        assert!(argv_de_texto("# solo comentarios\n", 8).is_err());
        assert!(argv_de_texto("a\nb\nc\n", 2).is_err());
    }
}

//! Llamadas a herramientas restringidas por gramática (salidas estructuradas).
//!
//! Hoy una llamada mal formada (nombre inexistente, JSON sin cerrar, clave que no es del esquema) se descubre
//! **después** de generar y el agente reintenta. Aquí la llamada se escribe byte a byte contra un autómata de
//! la familia de plantilla, y el muestreador sólo puede elegir tokens que lo avanzan: la llamada no puede salir
//! mal formada.
//!
//! El autómata es **perezoso**: hasta que el modelo escribe el marcador que abre la llamada (`<tool_call>`,
//! `<|tool_call>`) no restringe nada, y al cerrarla se suelta. Un marcador dentro de un razonamiento
//! (`<think>…</think>`, canal de Gemma) no la abre. Lo que cada familia admite es lo que ya entiende su parser:
//!
//! | familia | llamada |
//! |---|---|
//! | `qwen2`, `mellum` | `<tool_call>{"name": NOMBRE, "arguments": {…}}</tool_call>` |
//! | `gemma4`, `gemma4-pensar` | `<|tool_call>call:NOMBRE{clave:valor,…}<tool_call|>` (cadenas entre `<|"|>`) |
//! | `qwen3coder` | `<tool_call><function=NOMBRE><parameter=P>valor</parameter>…</function></tool_call>` |
//!
//! - `NOMBRE` sólo puede ser el de una herramienta declarada (la elegida si `tool_choice` es una concreta).
//! - Las claves de `arguments` sólo pueden ser las `properties` de la herramienta, una vez cada una, y la
//!   llamada no se cierra hasta tener las `required`. Sin `properties`, las claves son libres.
//! - Un valor sigue el `type` de su propiedad (cadena, entero, número, booleano); el resto (objetos, arreglos,
//!   sin tipo) es JSON cualquiera, con las cadenas bien escapadas.
//! - Si el autómata se queda sin salida (p. ej. un nombre que el vocabulario no puede escribir) se suelta y la
//!   generación sigue como sin restricción: no peor que antes.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use serde_json::Value;

use super::{ChatInput, ToolChoice, ToolDefinition};
use crate::restriccion::{Restriccion, TablaBytes};

const COMILLA_GEMMA: &[u8] = b"<|\"|>";
const CIERRA_PARAMETRO: &[u8] = b"</parameter>";
const PROFUNDIDAD_MAX: usize = 16;
const COLA_LIBRE: usize = 32;

/// Cómo se escriben cadenas, claves y nombres en la llamada.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dialecto {
    /// JSON (Qwen2, Mellum).
    Json,
    /// `call:nombre{clave:valor}` con cadenas entre `<|"|>` (Gemma 4).
    Gemma,
    /// `<function=…><parameter=…>` con el valor como texto (Qwen3-Coder).
    Xml,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tipo {
    Cadena,
    Entero,
    Numero,
    Booleano,
    Arreglo,
    Objeto,
    Cualquiera,
}

struct Clave {
    nombre: Vec<u8>,
    tipo: Tipo,
    requerida: bool,
}

struct Herramienta {
    nombre: Vec<u8>,
    claves: Vec<Clave>,
    /// Sin `properties`: cualquier clave.
    libre: bool,
}

enum Paso {
    /// Espacios en blanco opcionales.
    Ws,
    Lit(Vec<u8>),
    Nombre,
    Args,
    Xml,
}

impl Config {
    /// Marcadores que no pueden aparecer dentro de una llamada, de ninguna familia (un parser que busca texto
    /// se desviaría con cualquiera de ellos): salvo el cierre propio, que sólo vale si cierra de verdad.
    fn marcadores_ajenos(&self) -> impl Iterator<Item = &'static [u8]> + '_ {
        const TODOS: [&[u8]; 8] = [
            b"<tool_call>", b"</tool_call>", b"<|tool_call>", b"<tool_call|>", b"<|channel>", b"<channel|>",
            b"<think>", b"</think>",
        ];
        TODOS.into_iter().filter(|m| *m != self.cierra)
    }
}

fn contiene(haystack: &[u8], aguja: &[u8]) -> bool {
    !aguja.is_empty() && haystack.windows(aguja.len()).any(|w| w == aguja)
}

struct Config {
    dialecto: Dialecto,
    abre: &'static [u8],
    cierra: &'static [u8],
    pensar: Option<(&'static [u8], &'static [u8])>,
    pasos: Vec<Paso>,
    herramientas: Vec<Herramienta>,
}

fn es_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\t' | b'\r')
}

fn es_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

/// Resultado de dar un byte a una pieza del autómata.
#[derive(Clone, Copy, PartialEq, Eq)]
enum R {
    /// No cabe.
    No,
    /// Consumido.
    Come,
    /// La pieza ya había terminado: el byte es del siguiente paso.
    Sigue,
}

// ---------------------------------------------------------------------------------------------------------
// Valores
// ---------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cad {
    Normal,
    Esc,
    /// Dígitos hexadecimales que faltan de un `\uXXXX`.
    U(u8),
}

/// `Some(true)`: la cadena terminó con este byte; `Some(false)`: dentro; `None`: no cabe.
fn paso_cadena_json(c: &mut Cad, b: u8) -> Option<bool> {
    match *c {
        Cad::Normal => match b {
            b'"' => Some(true),
            b'\\' => {
                *c = Cad::Esc;
                Some(false)
            }
            0..=0x1f => None,
            _ => Some(false),
        },
        Cad::Esc => match b {
            b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                *c = Cad::Normal;
                Some(false)
            }
            b'u' => {
                *c = Cad::U(4);
                Some(false)
            }
            _ => None,
        },
        Cad::U(k) => {
            if b.is_ascii_hexdigit() {
                *c = if k <= 1 { Cad::Normal } else { Cad::U(k - 1) };
                Some(false)
            } else {
                None
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Num {
    Menos,
    Cero,
    Ent,
    Punto,
    Frac,
    Exp,
    ExpSigno,
    ExpDig,
}

#[derive(PartialEq, Eq)]
enum RNum {
    Come,
    /// El byte no es de este número, que ya estaba completo.
    Fin,
    No,
}

fn paso_numero(n: &mut Num, b: u8, entero: bool) -> RNum {
    let digito = b.is_ascii_digit();
    match *n {
        Num::Menos => match b {
            b'0' => {
                *n = Num::Cero;
                RNum::Come
            }
            b'1'..=b'9' => {
                *n = Num::Ent;
                RNum::Come
            }
            _ => RNum::No,
        },
        Num::Cero | Num::Ent | Num::Frac | Num::ExpDig => {
            let actual = *n;
            if digito {
                // Un cero inicial no admite más dígitos (`01` no es JSON).
                return if actual == Num::Cero { RNum::No } else { RNum::Come };
            }
            if !entero && b == b'.' && matches!(actual, Num::Cero | Num::Ent) {
                *n = Num::Punto;
                return RNum::Come;
            }
            if !entero && matches!(b, b'e' | b'E') && matches!(actual, Num::Cero | Num::Ent | Num::Frac) {
                *n = Num::Exp;
                return RNum::Come;
            }
            RNum::Fin
        }
        Num::Punto => {
            if digito {
                *n = Num::Frac;
                RNum::Come
            } else {
                RNum::No
            }
        }
        Num::Exp => {
            if matches!(b, b'+' | b'-') {
                *n = Num::ExpSigno;
                RNum::Come
            } else if digito {
                *n = Num::ExpDig;
                RNum::Come
            } else {
                RNum::No
            }
        }
        Num::ExpSigno => {
            if digito {
                *n = Num::ExpDig;
                RNum::Come
            } else {
                RNum::No
            }
        }
    }
}

#[derive(Clone)]
enum VS {
    /// Esperando el primer byte del valor.
    Inicio,
    /// Dentro de una cadena JSON (`true` si es una clave).
    Cad(Cad, bool),
    /// Dentro de una cadena de Gemma: bytes de `<|"|>` ya vistos del cierre.
    CadGemma(u8),
    /// Abriendo una cadena de Gemma: bytes de `<|"|>` ya vistos.
    AbreGemma(u8),
    Num(Num),
    /// Lo que falta de `true`, `false` o `null`.
    Lit(&'static [u8]),
    /// Dentro de un contenedor, tras un valor: `,` o el cierre.
    TrasValor,
    /// Tras `{`: clave o `}`.
    ObjAbre,
    /// Tras `,` en un objeto: clave.
    ObjClave,
    /// Tras `[`: valor o `]`.
    ArrAbre,
    /// Tras una clave JSON: `:`.
    DosPuntos,
    /// Clave de Gemma (sin comillas), con su longitud.
    ClaveGemma(usize),
    /// Valor completo.
    Fin,
}

/// Un valor JSON (o de Gemma) escrito byte a byte. El `tipo` sólo manda en el valor de nivel superior; lo que
/// hay dentro de un objeto o arreglo es cualquiera.
#[derive(Clone)]
struct Valor {
    dial: Dialecto,
    tipo: Tipo,
    st: VS,
    pila: Vec<u8>,
    /// Secuencia de bytes que no puede aparecer en ningún sitio del valor (el cierre de la etiqueta que lo
    /// contiene) y los bytes de ella ya vistos.
    veto: &'static [u8],
    vm: u8,
    /// ¿Se admiten espacios antes del valor? (En XML el parser sólo quita un salto de línea.)
    ws_inicio: bool,
}

impl Valor {
    fn nuevo(dial: Dialecto, tipo: Tipo) -> Self {
        Valor { dial, tipo, st: VS::Inicio, pila: Vec::new(), veto: &[], vm: 0, ws_inicio: true }
    }

    /// Valor que va entre etiquetas XML: sin espacios alrededor (sólo el salto de línea que el parser quita) y
    /// sin la secuencia que cierra la etiqueta.
    fn entre_etiquetas(mut self, veto: &'static [u8]) -> Self {
        self.veto = veto;
        self.ws_inicio = false;
        self
    }

    fn fin_valor(&mut self) {
        self.st = if self.pila.is_empty() { VS::Fin } else { VS::TrasValor };
    }

    fn inicio(&mut self, b: u8) -> R {
        let t = if self.pila.is_empty() { self.tipo } else { Tipo::Cualquiera };
        let cadena = matches!(t, Tipo::Cadena | Tipo::Cualquiera);
        let numero = matches!(t, Tipo::Entero | Tipo::Numero | Tipo::Cualquiera);
        let booleano = matches!(t, Tipo::Booleano | Tipo::Cualquiera);
        let cualquiera = t == Tipo::Cualquiera;
        let objeto = matches!(t, Tipo::Objeto | Tipo::Cualquiera);
        let arreglo = matches!(t, Tipo::Arreglo | Tipo::Cualquiera);
        match b {
            b'"' if self.dial == Dialecto::Json && cadena => self.st = VS::Cad(Cad::Normal, false),
            b'<' if self.dial == Dialecto::Gemma && cadena => self.st = VS::AbreGemma(1),
            b'{' if objeto && self.pila.len() < PROFUNDIDAD_MAX => {
                self.pila.push(b'{');
                self.st = VS::ObjAbre;
            }
            b'[' if arreglo && self.pila.len() < PROFUNDIDAD_MAX => {
                self.pila.push(b'[');
                self.st = VS::ArrAbre;
            }
            b'-' if numero => self.st = VS::Num(Num::Menos),
            b'0' if numero => self.st = VS::Num(Num::Cero),
            b'1'..=b'9' if numero => self.st = VS::Num(Num::Ent),
            b't' if booleano => self.st = VS::Lit(b"rue"),
            b'f' if booleano => self.st = VS::Lit(b"alse"),
            b'n' if cualquiera => self.st = VS::Lit(b"ull"),
            _ => return R::No,
        }
        R::Come
    }

    fn paso(&mut self, b: u8) -> R {
        if !self.veto.is_empty() {
            let m = self.vm as usize;
            let nuevo = if b == self.veto[m] {
                m + 1
            } else if b == self.veto[0] {
                1
            } else {
                0
            };
            if nuevo == self.veto.len() {
                return R::No;
            }
            self.vm = nuevo as u8;
        }
        // Bucle: algunos estados terminan «sin consumir» y el byte lo recibe el siguiente.
        loop {
            match self.st {
                VS::Fin => return R::Sigue,
                VS::Inicio => {
                    if es_ws(b) && self.ws_inicio {
                        return R::Come;
                    }
                    return self.inicio(b);
                }
                VS::Cad(ref mut c, es_clave) => {
                    return match paso_cadena_json(c, b) {
                        None => R::No,
                        Some(false) => R::Come,
                        Some(true) => {
                            if es_clave {
                                self.st = VS::DosPuntos;
                            } else {
                                self.fin_valor();
                            }
                            R::Come
                        }
                    };
                }
                VS::AbreGemma(k) => {
                    if COMILLA_GEMMA[k as usize] != b {
                        return R::No;
                    }
                    self.st = if k as usize + 1 == COMILLA_GEMMA.len() { VS::CadGemma(0) } else { VS::AbreGemma(k + 1) };
                    return R::Come;
                }
                VS::CadGemma(m) => {
                    let mut m = m as usize;
                    if b == COMILLA_GEMMA[m] {
                        m += 1;
                    } else if b == COMILLA_GEMMA[0] {
                        m = 1;
                    } else {
                        m = 0;
                    }
                    if m == COMILLA_GEMMA.len() {
                        self.fin_valor();
                    } else {
                        self.st = VS::CadGemma(m as u8);
                    }
                    return R::Come;
                }
                VS::Num(ref mut n) => {
                    let entero = self.pila.is_empty() && self.tipo == Tipo::Entero;
                    match paso_numero(n, b, entero) {
                        RNum::Come => return R::Come,
                        RNum::No => return R::No,
                        RNum::Fin => {
                            // Sólo termina un número en un estado completo; `paso_numero` ya lo garantiza.
                            self.fin_valor();
                            continue;
                        }
                    }
                }
                VS::Lit(resto) => {
                    if resto.first() != Some(&b) {
                        return R::No;
                    }
                    if resto.len() == 1 {
                        self.fin_valor();
                    } else {
                        self.st = VS::Lit(&resto[1..]);
                    }
                    return R::Come;
                }
                VS::ObjAbre | VS::ObjClave => {
                    if es_ws(b) {
                        return R::Come;
                    }
                    if b == b'}' && matches!(self.st, VS::ObjAbre) {
                        self.pila.pop();
                        self.fin_valor();
                        return R::Come;
                    }
                    return match self.dial {
                        Dialecto::Gemma if es_ident(b) => {
                            self.st = VS::ClaveGemma(1);
                            R::Come
                        }
                        Dialecto::Json if b == b'"' => {
                            self.st = VS::Cad(Cad::Normal, true);
                            R::Come
                        }
                        _ => R::No,
                    };
                }
                VS::ClaveGemma(n) => {
                    if es_ident(b) {
                        self.st = VS::ClaveGemma(n + 1);
                    } else if b == b':' {
                        self.st = VS::Inicio;
                    } else {
                        return R::No;
                    }
                    return R::Come;
                }
                VS::DosPuntos => {
                    if es_ws(b) {
                        return R::Come;
                    }
                    if b == b':' {
                        self.st = VS::Inicio;
                        return R::Come;
                    }
                    return R::No;
                }
                VS::ArrAbre => {
                    if es_ws(b) {
                        return R::Come;
                    }
                    if b == b']' {
                        self.pila.pop();
                        self.fin_valor();
                        return R::Come;
                    }
                    self.st = VS::Inicio;
                    continue;
                }
                VS::TrasValor => {
                    if es_ws(b) {
                        return R::Come;
                    }
                    let tope = *self.pila.last().unwrap_or(&0);
                    match (b, tope) {
                        (b',', b'{') => self.st = VS::ObjClave,
                        (b',', b'[') => self.st = VS::Inicio,
                        (b'}', b'{') | (b']', b'[') => {
                            self.pila.pop();
                            self.fin_valor();
                        }
                        _ => return R::No,
                    }
                    return R::Come;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Argumentos (objeto con las claves del esquema)
// ---------------------------------------------------------------------------------------------------------

#[derive(Clone)]
enum AS {
    /// Esperando `{`.
    Abre,
    /// Tras `{` (`primera`) o tras `,`: clave o, sólo en el primer caso y con las requeridas, `}`.
    ClaveOCierre { primera: bool },
    /// Clave del esquema, con lo escrito hasta ahora.
    Clave(Vec<u8>),
    /// Clave libre JSON.
    ClaveLibre(Cad),
    /// Clave libre de Gemma.
    ClaveLibreGemma(usize),
    DosPuntos,
    Valor(Valor),
    TrasValor,
    Hecho,
}

/// Claves sin usar de `h` que empiezan por `prefijo`.
fn candidatas<'a>(h: &'a Herramienta, usadas: &'a [bool], prefijo: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    h.claves
        .iter()
        .enumerate()
        .filter(move |(i, c)| !usadas[*i] && c.nombre.starts_with(prefijo))
        .map(|(i, _)| i)
}

#[derive(Clone)]
struct Args {
    herr: usize,
    usadas: Vec<bool>,
    clave: Option<usize>,
    st: AS,
}

impl Args {
    fn nuevo(cfg: &Config, herr: usize) -> Self {
        Args { herr, usadas: alloc::vec![false; cfg.herramientas[herr].claves.len()], clave: None, st: AS::Abre }
    }

    fn requeridas_ok(&self, h: &Herramienta) -> bool {
        h.claves.iter().zip(&self.usadas).all(|(c, &u)| !c.requerida || u)
    }

    fn quedan_claves(&self, h: &Herramienta) -> bool {
        h.libre || self.usadas.iter().any(|&u| !u)
    }

    fn abre_valor(&mut self, cfg: &Config, idx: Option<usize>) {
        let h = &cfg.herramientas[self.herr];
        let tipo = idx.map_or(Tipo::Cualquiera, |i| h.claves[i].tipo);
        self.clave = idx;
        self.st = AS::Valor(Valor::nuevo(cfg.dialecto, tipo));
    }

    fn paso(&mut self, cfg: &Config, b: u8) -> R {
        let h = &cfg.herramientas[self.herr];
        let json = cfg.dialecto == Dialecto::Json;
        loop {
            match &mut self.st {
                AS::Hecho => return R::Sigue,
                AS::Abre => {
                    if b != b'{' {
                        return R::No;
                    }
                    self.st = AS::ClaveOCierre { primera: true };
                    return R::Come;
                }
                AS::ClaveOCierre { primera } => {
                    if es_ws(b) {
                        return R::Come;
                    }
                    let primera = *primera;
                    if b == b'}' && primera {
                        if self.requeridas_ok(h) {
                            self.st = AS::Hecho;
                            return R::Come;
                        }
                        return R::No;
                    }
                    if json {
                        if b != b'"' {
                            return R::No;
                        }
                        if h.libre {
                            self.st = AS::ClaveLibre(Cad::Normal);
                        } else if self.usadas.iter().any(|&u| !u) {
                            self.st = AS::Clave(Vec::new());
                        } else {
                            return R::No;
                        }
                        return R::Come;
                    }
                    // Gemma: la clave va sin comillas, así que este byte ya es de ella.
                    if h.libre {
                        if !es_ident(b) {
                            return R::No;
                        }
                        self.st = AS::ClaveLibreGemma(1);
                        return R::Come;
                    }
                    self.st = AS::Clave(Vec::new());
                    continue;
                }
                AS::Clave(prefijo) => {
                    let cierre = if json { b'"' } else { b':' };
                    if b == cierre {
                        let p = core::mem::take(prefijo);
                        let completa = candidatas(h, &self.usadas, &p).find(|&i| h.claves[i].nombre == p);
                        let Some(i) = completa else {
                            return R::No;
                        };
                        if json {
                            self.clave = Some(i);
                            self.st = AS::DosPuntos;
                        } else {
                            self.abre_valor(cfg, Some(i));
                        }
                        return R::Come;
                    }
                    let mut siguiente = prefijo.clone();
                    siguiente.push(b);
                    if candidatas(h, &self.usadas, &siguiente).next().is_none() {
                        return R::No;
                    }
                    *prefijo = siguiente;
                    return R::Come;
                }
                AS::ClaveLibre(c) => {
                    return match paso_cadena_json(c, b) {
                        None => R::No,
                        Some(false) => R::Come,
                        Some(true) => {
                            self.clave = None;
                            self.st = AS::DosPuntos;
                            R::Come
                        }
                    };
                }
                AS::ClaveLibreGemma(n) => {
                    if es_ident(b) {
                        *n += 1;
                    } else if b == b':' {
                        self.abre_valor(cfg, None);
                    } else {
                        return R::No;
                    }
                    return R::Come;
                }
                AS::DosPuntos => {
                    if es_ws(b) {
                        return R::Come;
                    }
                    if b != b':' {
                        return R::No;
                    }
                    let idx = self.clave;
                    self.abre_valor(cfg, idx);
                    return R::Come;
                }
                AS::Valor(v) => match v.paso(b) {
                    R::No => return R::No,
                    R::Come => return R::Come,
                    R::Sigue => {
                        if let Some(i) = self.clave.take() {
                            self.usadas[i] = true;
                        }
                        self.st = AS::TrasValor;
                        continue;
                    }
                },
                AS::TrasValor => {
                    if es_ws(b) {
                        return R::Come;
                    }
                    if b == b',' && self.quedan_claves(h) {
                        self.st = AS::ClaveOCierre { primera: false };
                        return R::Come;
                    }
                    if b == b'}' && self.requeridas_ok(h) {
                        self.st = AS::Hecho;
                        return R::Come;
                    }
                    return R::No;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Parámetros XML (Qwen3-Coder)
// ---------------------------------------------------------------------------------------------------------

#[derive(Clone)]
enum XS {
    /// Entre parámetros: espacios, `<parameter=` o `</function>`; lo ya escrito de la etiqueta.
    Entre(Vec<u8>),
    /// Nombre del parámetro, hasta `>`.
    Clave(Vec<u8>),
    /// Texto del valor, hasta `</parameter>` (bytes ya vistos del cierre) y la clave a la que pertenece.
    Valor(usize, Option<usize>),
    /// Tras `<parameter=P>`: un salto de línea opcional y el valor tipado.
    TipadoInicio(Tipo, usize),
    /// Valor que no es texto (número, booleano, arreglo, objeto): se escribe como JSON.
    Tipado(Valor, usize),
    /// Tras un valor tipado: un salto de línea opcional (`bool`: ya está) y `</parameter>` (bytes ya vistos).
    Cierre(usize, usize, bool),
    Hecho,
}

#[derive(Clone)]
struct Xml {
    herr: usize,
    usadas: Vec<bool>,
    st: XS,
}

impl Xml {
    fn nuevo(cfg: &Config, herr: usize) -> Self {
        Xml { herr, usadas: alloc::vec![false; cfg.herramientas[herr].claves.len()], st: XS::Entre(Vec::new()) }
    }

    fn paso(&mut self, cfg: &Config, b: u8) -> R {
        const ABRE: &[u8] = b"<parameter=";
        const CIERRA: &[u8] = b"</function>";
        let h = &cfg.herramientas[self.herr];
        let requeridas_ok = h.claves.iter().zip(&self.usadas).all(|(c, &u)| !c.requerida || u);
        let quedan = h.libre || self.usadas.iter().any(|&u| !u);
        match &mut self.st {
            XS::Hecho => R::Sigue,
            XS::Entre(buf) => {
                if buf.is_empty() && es_ws(b) {
                    return R::Come;
                }
                buf.push(b);
                let abre = quedan && ABRE.starts_with(buf);
                let cierra = requeridas_ok && CIERRA.starts_with(buf);
                if !abre && !cierra {
                    return R::No;
                }
                if abre && buf.len() == ABRE.len() {
                    self.st = XS::Clave(Vec::new());
                } else if cierra && buf.len() == CIERRA.len() {
                    self.st = XS::Hecho;
                }
                R::Come
            }
            XS::Clave(prefijo) => {
                if b == b'>' {
                    let idx = h.claves.iter().enumerate().position(|(i, c)| !self.usadas[i] && c.nombre == *prefijo);
                    return match idx {
                        Some(i) => {
                            self.st = match h.claves[i].tipo {
                                Tipo::Cadena | Tipo::Cualquiera => XS::Valor(0, Some(i)),
                                t => XS::TipadoInicio(t, i),
                            };
                            R::Come
                        }
                        None if h.libre && !prefijo.is_empty() => {
                            self.st = XS::Valor(0, None);
                            R::Come
                        }
                        None => R::No,
                    };
                }
                let mut siguiente = prefijo.clone();
                siguiente.push(b);
                let cabe = if h.libre {
                    !matches!(b, b'<' | b'\n' | b'\r')
                } else {
                    h.claves.iter().enumerate().any(|(i, c)| !self.usadas[i] && c.nombre.starts_with(&siguiente))
                };
                if !cabe {
                    return R::No;
                }
                *prefijo = siguiente;
                R::Come
            }
            XS::TipadoInicio(t, i) => {
                let valor = Valor::nuevo(Dialecto::Json, *t).entre_etiquetas(CIERRA_PARAMETRO);
                let i = *i;
                self.st = XS::Tipado(valor, i);
                if b == b'\n' {
                    return R::Come;
                }
                self.paso(cfg, b)
            }
            XS::Tipado(v, i) => match v.paso(b) {
                R::No => R::No,
                R::Come => R::Come,
                R::Sigue => {
                    self.st = XS::Cierre(*i, 0, false);
                    self.paso(cfg, b)
                }
            },
            XS::Cierre(i, k, nl) => {
                if *k == 0 && !*nl && b == b'\n' {
                    *nl = true;
                    return R::Come;
                }
                if CIERRA_PARAMETRO[*k] != b {
                    return R::No;
                }
                *k += 1;
                if *k == CIERRA_PARAMETRO.len() {
                    self.usadas[*i] = true;
                    self.st = XS::Entre(Vec::new());
                }
                R::Come
            }
            XS::Valor(m, clave) => {
                if b == CIERRA_PARAMETRO[*m] {
                    *m += 1;
                } else if b == CIERRA_PARAMETRO[0] {
                    *m = 1;
                } else {
                    *m = 0;
                }
                if *m == CIERRA_PARAMETRO.len() {
                    if let Some(i) = *clave {
                        self.usadas[i] = true;
                    }
                    self.st = XS::Entre(Vec::new());
                }
                R::Come
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// La llamada entera
// ---------------------------------------------------------------------------------------------------------

#[derive(Clone)]
enum Est {
    Nada,
    Lit(usize),
    Nombre(Vec<u8>),
    Args(Args),
    Xml(Xml),
}

#[derive(Clone)]
struct Llamada {
    idx: usize,
    est: Est,
    herr: usize,
    hecha: bool,
}

impl Llamada {
    fn nueva(cfg: &Config) -> Self {
        let mut l = Llamada { idx: 0, est: Est::Nada, herr: usize::MAX, hecha: false };
        l.entrar(cfg);
        l
    }

    fn entrar(&mut self, cfg: &Config) {
        match cfg.pasos.get(self.idx) {
            None => self.hecha = true,
            Some(Paso::Ws) => self.est = Est::Nada,
            Some(Paso::Lit(_)) => self.est = Est::Lit(0),
            Some(Paso::Nombre) => self.est = Est::Nombre(Vec::new()),
            Some(Paso::Args) => self.est = Est::Args(Args::nuevo(cfg, self.herr)),
            Some(Paso::Xml) => self.est = Est::Xml(Xml::nuevo(cfg, self.herr)),
        }
    }

    fn avanzar(&mut self, cfg: &Config) {
        self.idx += 1;
        self.entrar(cfg);
    }

    /// `false` si el byte no cabe.
    fn paso(&mut self, cfg: &Config, b: u8) -> bool {
        loop {
            if self.hecha {
                return true;
            }
            let r = match (&cfg.pasos[self.idx], &mut self.est) {
                (Paso::Ws, _) => {
                    if es_ws(b) {
                        R::Come
                    } else {
                        R::Sigue
                    }
                }
                (Paso::Lit(lit), Est::Lit(p)) => {
                    if lit[*p] != b {
                        return false;
                    }
                    *p += 1;
                    if *p == lit.len() {
                        self.avanzar(cfg);
                    }
                    return true;
                }
                (Paso::Nombre, Est::Nombre(prefijo)) => {
                    let mut siguiente = prefijo.clone();
                    siguiente.push(b);
                    if cfg.herramientas.iter().any(|h| h.nombre.starts_with(&siguiente)) {
                        *prefijo = siguiente;
                        R::Come
                    } else if let Some(i) = cfg.herramientas.iter().position(|h| h.nombre == *prefijo) {
                        self.herr = i;
                        R::Sigue
                    } else {
                        R::No
                    }
                }
                (Paso::Args, Est::Args(a)) => a.paso(cfg, b),
                (Paso::Xml, Est::Xml(x)) => x.paso(cfg, b),
                _ => R::No,
            };
            match r {
                R::No => return false,
                R::Come => return true,
                R::Sigue => self.avanzar(cfg),
            }
        }
    }
}

#[derive(Clone)]
enum Fase {
    /// Sin llamada abierta: se vigilan los últimos bytes en busca del marcador.
    Libre { cola: Vec<u8>, pensando: bool },
    Llamada(Llamada),
    /// Llamada cerrada.
    Hecha,
    Abandonada,
}

/// Restricción de una llamada a herramienta; ver el módulo.
#[derive(Clone)]
pub struct GramaticaLlamada {
    cfg: Arc<Config>,
    tabla: Arc<TablaBytes>,
    fase: Fase,
}

impl GramaticaLlamada {
    fn byte(&mut self, b: u8) -> bool {
        let cfg: &Config = &self.cfg;
        match &mut self.fase {
            Fase::Libre { cola, pensando } => {
                cola.push(b);
                if cola.len() > COLA_LIBRE {
                    cola.remove(0);
                }
                if let Some((abre, cierra)) = cfg.pensar {
                    if cola.ends_with(abre) {
                        *pensando = true;
                    } else if cola.ends_with(cierra) {
                        *pensando = false;
                    }
                }
                if !*pensando && cola.ends_with(cfg.abre) {
                    self.fase = Fase::Llamada(Llamada::nueva(cfg));
                }
                true
            }
            Fase::Llamada(l) => {
                if !l.paso(cfg, b) {
                    return false;
                }
                if l.hecha {
                    self.fase = Fase::Hecha;
                }
                true
            }
            Fase::Hecha | Fase::Abandonada => true,
        }
    }
}

impl Restriccion for GramaticaLlamada {
    fn permite(&self, token: u32) -> bool {
        if !matches!(self.fase, Fase::Llamada(_)) {
            return true;
        }
        let bytes = self.tabla.bytes(token);
        // Un token que no emite nada (fin de secuencia) no cabe a mitad de una llamada.
        if bytes.is_empty() {
            return false;
        }
        // Los marcadores escritos **dentro** de la llamada (en una cadena, en un nombre) harían que los parsers
        // que buscan texto la partan o la descarten. Ninguno tiene sitio en una llamada; el cierre propio sólo
        // vale si es el que la termina.
        let cierra = contiene(bytes, self.cfg.cierra);
        if self.cfg.marcadores_ajenos().any(|m| contiene(bytes, m)) {
            return false;
        }
        let mut copia = self.clone();
        if !bytes.iter().all(|&b| copia.byte(b)) {
            return false;
        }
        !cierra || matches!(copia.fase, Fase::Hecha)
    }

    fn avanza(&mut self, token: u32) {
        let tabla = self.tabla.clone();
        let bytes = tabla.bytes(token);
        if bytes.is_empty() {
            if matches!(self.fase, Fase::Llamada(_)) {
                self.fase = Fase::Abandonada;
            }
            return;
        }
        for &b in bytes {
            if !self.byte(b) {
                self.fase = Fase::Abandonada;
                return;
            }
        }
    }

    fn activa(&self) -> bool {
        matches!(self.fase, Fase::Llamada(_))
    }

    fn abandonar(&mut self) {
        self.fase = Fase::Abandonada;
    }

    fn duplicar(&self) -> Box<dyn Restriccion> {
        Box::new(self.clone())
    }
}

// ---------------------------------------------------------------------------------------------------------
// Construcción desde las herramientas declaradas
// ---------------------------------------------------------------------------------------------------------

fn texto_json_sin_comillas(s: &str) -> Vec<u8> {
    let j = serde_json::to_string(s).unwrap_or_default();
    j.as_bytes().get(1..j.len().saturating_sub(1)).unwrap_or(&[]).to_vec()
}

fn tipo_de(prop: &Value) -> Tipo {
    match prop.get("type").and_then(Value::as_str) {
        Some("string") => Tipo::Cadena,
        Some("integer") => Tipo::Entero,
        Some("number") => Tipo::Numero,
        Some("boolean") => Tipo::Booleano,
        Some("array") => Tipo::Arreglo,
        Some("object") => Tipo::Objeto,
        _ => Tipo::Cualquiera,
    }
}

fn herramienta(def: &ToolDefinition, dial: Dialecto) -> Herramienta {
    let nombre_de = |s: &str| if dial == Dialecto::Json { texto_json_sin_comillas(s) } else { s.as_bytes().to_vec() };
    let mut claves: Vec<Clave> = Vec::new();
    let anadir = |claves: &mut Vec<Clave>, nombre: &str, tipo: Tipo, requerida: bool| {
        let n = nombre_de(nombre);
        match claves.iter_mut().find(|c| c.nombre == n) {
            Some(c) => c.requerida |= requerida,
            None => claves.push(Clave { nombre: n, tipo, requerida }),
        }
    };
    if let Some(props) = def.parameters.get("properties").and_then(Value::as_object) {
        for (k, v) in props {
            anadir(&mut claves, k, tipo_de(v), false);
        }
    }
    if let Some(req) = def.parameters.get("required").and_then(Value::as_array) {
        for r in req.iter().filter_map(Value::as_str) {
            anadir(&mut claves, r, Tipo::Cualquiera, true);
        }
    }
    Herramienta { nombre: nombre_de(&def.name), libre: claves.is_empty(), claves }
}

fn lit(s: &str) -> Paso {
    Paso::Lit(s.as_bytes().to_vec())
}

/// Restricción para las llamadas a herramienta de `entrada`, o `None` si no hay nada que restringir (sin
/// herramientas, `tool_choice: none`, la herramienta pedida no existe o la familia no tiene gramática).
///
/// `pensando`: la generación empieza dentro de un razonamiento ya abierto por el aviso de generación.
pub fn restriccion_de_herramientas(
    familia: &str,
    entrada: &ChatInput,
    tabla: Arc<TablaBytes>,
    pensando: bool,
) -> Option<Box<dyn Restriccion>> {
    if entrada.tools.is_empty() || entrada.tool_choice == ToolChoice::None {
        return None;
    }
    let dialecto = match familia {
        "qwen2" | "mellum" => Dialecto::Json,
        "gemma4" | "gemma4-pensar" => Dialecto::Gemma,
        "qwen3coder" => Dialecto::Xml,
        _ => return None,
    };
    let definiciones: Vec<&ToolDefinition> = match &entrada.tool_choice {
        ToolChoice::Named(n) => entrada.tools.iter().filter(|t| &t.name == n).collect(),
        _ => entrada.tools.iter().collect(),
    };
    if definiciones.is_empty() {
        return None;
    }
    let herramientas: Vec<Herramienta> = definiciones.into_iter().map(|d| herramienta(d, dialecto)).collect();
    let (abre, cierra, pensar, pasos): (&'static [u8], &'static [u8], Option<(&'static [u8], &'static [u8])>, Vec<Paso>) =
        match dialecto {
        Dialecto::Json => (
            b"<tool_call>",
            b"</tool_call>",
            Some((b"<think>", b"</think>")),
            alloc::vec![
                Paso::Ws,
                lit("{"),
                Paso::Ws,
                lit("\"name\""),
                Paso::Ws,
                lit(":"),
                Paso::Ws,
                lit("\""),
                Paso::Nombre,
                lit("\""),
                Paso::Ws,
                lit(","),
                Paso::Ws,
                lit("\"arguments\""),
                Paso::Ws,
                lit(":"),
                Paso::Ws,
                Paso::Args,
                Paso::Ws,
                lit("}"),
                Paso::Ws,
                lit("</tool_call>"),
            ],
        ),
        Dialecto::Gemma => (
            b"<|tool_call>",
            b"<tool_call|>",
            Some((b"<|channel>", b"<channel|>")),
            alloc::vec![Paso::Ws, lit("call:"), Paso::Nombre, Paso::Ws, Paso::Args, Paso::Ws, lit("<tool_call|>")],
        ),
        Dialecto::Xml => (
            b"<tool_call>",
            b"</tool_call>",
            Some((b"<think>", b"</think>")),
            alloc::vec![Paso::Ws, lit("<function="), Paso::Nombre, lit(">"), Paso::Xml, Paso::Ws, lit("</tool_call>")],
        ),
    };
    let cfg = Arc::new(Config { dialecto, abre, cierra, pensar, pasos, herramientas });
    Some(Box::new(GramaticaLlamada { cfg, tabla, fase: Fase::Libre { cola: Vec::new(), pensando } }))
}

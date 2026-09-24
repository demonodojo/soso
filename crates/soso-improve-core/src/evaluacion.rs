//! Evaluación del modelo: medir antes de confiar (T14).
//!
//! La pregunta que contesta este módulo es «¿sirve este modelo para lo que el
//! plan quiere hacer con él?», y la contesta con un **go/no-go** medido, no con
//! una impresión. Lo que lo hace fiable son tres reglas incómodas:
//!
//! 1. **Todos los intentos cuentan en el denominador.** Un caso que falla dos
//!    veces y acierta a la tercera no es un caso que pasa: es un caso que pasa
//!    un tercio de las veces. Contar solo el mejor intento es cómo se fabrica
//!    un go que luego no se sostiene.
//! 2. **El verificador no ve lo que ve el modelo.** Las aserciones viven en la
//!    partición reservada y se aplican fuera del contexto del modelo, así que
//!    no hay forma de «acertar» copiando el criterio.
//! 3. **Lo que no se puede medir no se apunta como medido.** Si un caso espera
//!    una generación y el endpoint no devuelve `usage`, el intento se marca
//!    `SinUso` y **no cuenta como éxito**, por muy bien que se lea la
//!    respuesta. Quién lo exige lo decide el **caso** (`exige_uso`), no el
//!    evaluador: un 400 o un flujo SSE no tienen por qué traerlo, y pedírselo
//!    convierte un acierto en un fallo inventado.
//!
//! El reloj y los plazos entran por [`crate::tiempo`], así que las pruebas no
//! dependen de dormir de verdad y la medida no depende de la hora del día.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::tiempo::{Plazo, Reloj};
use crate::{Error, Resultado, ESQUEMA};

/// Lo que el evaluador necesita de un endpoint, sea host o guest.
///
/// Deliberadamente **no** habla HTTP: el transporte y el codec ya existen
/// (T10–T12, T48) y duplicarlos aquí sería inventar una tercera versión que se
/// desincroniza. Aquí solo entra «manda esta petición grabada y devuélveme lo
/// que conteste».
pub trait ClienteModelo {
    /// `peticion` es el documento JSON grabado del caso. Devuelve la respuesta
    /// cruda del endpoint y lo que se pudo medir de ella.
    fn completar(&mut self, peticion: &str, plazo: Plazo) -> Resultado<Respuesta>;
}

/// Respuesta de un intento, con lo medido al lado de lo dicho.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Respuesta {
    /// Documento JSON tal cual lo devolvió el endpoint.
    pub documento: String,
    /// Milisegundos monotónicos hasta el primer byte útil.
    pub ms_primer_byte: u64,
    /// Hasta el primer token de contenido. Cero si no se pudo distinguir.
    pub ms_primer_token: u64,
    /// Hasta el final de la respuesta.
    pub ms_total: u64,
    /// Tokens que **el endpoint** dice haber gastado. `None` = no lo dijo, y
    /// eso no se puede rellenar con una estimación sin mentir.
    pub tokens_entrada: Option<u32>,
    pub tokens_salida: Option<u32>,
    /// Memoria que reporte el servidor, si la reporta.
    pub memoria_mb: Option<u32>,
}

impl Respuesta {
    pub fn tiene_uso(&self) -> bool {
        self.tokens_entrada.is_some() && self.tokens_salida.is_some()
    }
}

/// Cómo acabó un intento. Son estados distintos a propósito: agruparlos en
/// «falló» esconde exactamente la información que hace falta para corregir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Desenlace {
    /// Verificado por el comprobador reservado.
    Bien,
    /// Contestó, y el comprobador dice que no.
    Mal,
    /// No contestó dentro del plazo.
    Plazo,
    /// Contestó, pero sin `usage`: no se puede acreditar el consumo.
    SinUso,
    /// El endpoint o el transporte fallaron.
    Error,
}

impl Desenlace {
    pub fn nombre(self) -> &'static str {
        match self {
            Desenlace::Bien => "bien",
            Desenlace::Mal => "mal",
            Desenlace::Plazo => "plazo",
            Desenlace::SinUso => "sin-uso",
            Desenlace::Error => "error",
        }
    }

    /// **Solo** `Bien` cuenta. Lo demás, no, por muy explicable que sea.
    pub fn cuenta_como_exito(self) -> bool {
        self == Desenlace::Bien
    }
}

/// Un intento concreto, con su semilla para poder repetirlo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intento {
    pub caso: String,
    pub repeticion: u32,
    pub semilla: u64,
    /// El primer intento de cada caso es «frío»: el servidor puede no tener
    /// nada cacheado. Mezclarlo con los calientes ensucia la medida de
    /// velocidad, aunque para la corrección cuenten igual.
    pub frio: bool,
    pub desenlace: Desenlace,
    pub ms_primer_byte: u64,
    pub ms_primer_token: u64,
    pub ms_total: u64,
    pub tokens_salida: Option<u32>,
    pub detalle: String,
    /// El documento que devolvió el endpoint, recortado.
    ///
    /// Sin esto, un fallo sólo deja las aserciones que no pasaron, y eso no
    /// basta para atribuirlo: en la campaña de T14 hubo que dejar un caso «sin
    /// determinar» porque el contenido crudo se había perdido. Guardarlo es
    /// barato y es la diferencia entre diagnosticar y adivinar.
    pub documento: String,
}

/// Tope de lo que se guarda de cada respuesta. Suficiente para ver qué dijo el
/// modelo sin convertir el informe en un volcado.
pub const DOCUMENTO_MAX: usize = 4096;

impl Intento {
    pub fn ok(&self) -> bool {
        self.desenlace.cuenta_como_exito()
    }
}

/// Resumen de una clase de casos (protocolo, programación…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumenClase {
    pub clase: String,
    /// Casos que salieron bien **en todos** sus intentos.
    pub casos_solidos: usize,
    /// Casos que salieron bien al menos una vez pero no siempre. Se informan
    /// aparte porque un modelo que acierta a veces no es un modelo que acierta.
    pub casos_inestables: usize,
    pub casos_totales: usize,
    pub intentos_bien: usize,
    pub intentos_totales: usize,
}

impl ResumenClase {
    /// Casos aprobados con el criterio estricto del plan: todos los intentos.
    pub fn aprobados(&self) -> usize {
        self.casos_solidos
    }
}

/// Umbrales del plan. Se pasan explícitos para que una campaña no pueda
/// aprobarse bajándolos sin que se vea en el informe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Umbrales {
    /// Casos de protocolo que deben pasar, de los que haya.
    pub protocolo_minimo: usize,
    /// Microtareas de programación que deben pasar.
    pub programacion_minimo: usize,
    /// Repeticiones por caso.
    pub repeticiones: u32,
}

impl Default for Umbrales {
    fn default() -> Self {
        // SELF_IMPROVEMENT.md: 10/10 de protocolo y ≥8/10 de microtareas.
        Umbrales {
            protocolo_minimo: 10,
            programacion_minimo: 8,
            repeticiones: 3,
        }
    }
}

/// El veredicto y por qué.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Informe {
    pub schema_version: u32,
    pub modelo: String,
    pub banco: String,
    pub umbrales: Umbrales,
    pub clases: Vec<ResumenClase>,
    pub intentos: Vec<Intento>,
    /// Presupuestos medidos, para escribirlos en el model-lock.
    pub presupuesto: Presupuesto,
    pub go: bool,
    /// Qué falta exactamente para que sea go. Vacío si ya lo es.
    pub motivos: Vec<String>,
}

/// Lo que cuesta una tarea, medido y no estimado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Presupuesto {
    pub ms_primer_token_mediana: u64,
    pub ms_total_mediana: u64,
    pub ms_total_maximo: u64,
    /// Con qué plazo por petición se queda corto menos de lo tolerable: el
    /// máximo observado con holgura, no un número redondo elegido a ojo.
    pub timeout_sugerido_ms: u64,
    pub tokens_salida_maximo: u32,
}

impl Informe {
    pub fn clase(&self, nombre: &str) -> Option<&ResumenClase> {
        self.clases.iter().find(|c| c.clase == nombre)
    }

    /// JSON estable, a mano porque el crate es `no_std`.
    pub fn json(&self) -> String {
        let mut s = format!(
            "{{\"schema_version\":{},\"modelo\":\"{}\",\"banco\":\"{}\",\"go\":{},\"clases\":[",
            self.schema_version,
            escapar(&self.modelo),
            escapar(&self.banco),
            self.go
        );
        for (i, c) in self.clases.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"clase\":\"{}\",\"solidos\":{},\"inestables\":{},\"total\":{},\"intentos_bien\":{},\"intentos\":{}}}",
                escapar(&c.clase),
                c.casos_solidos,
                c.casos_inestables,
                c.casos_totales,
                c.intentos_bien,
                c.intentos_totales
            ));
        }
        s.push_str(&format!(
            "],\"presupuesto\":{{\"ms_primer_token_mediana\":{},\"ms_total_mediana\":{},\"ms_total_maximo\":{},\"timeout_sugerido_ms\":{},\"tokens_salida_maximo\":{}}},\"motivos\":[",
            self.presupuesto.ms_primer_token_mediana,
            self.presupuesto.ms_total_mediana,
            self.presupuesto.ms_total_maximo,
            self.presupuesto.timeout_sugerido_ms,
            self.presupuesto.tokens_salida_maximo
        ));
        for (i, m) in self.motivos.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push('"');
            s.push_str(&escapar(m));
            s.push('"');
        }
        s.push_str("]}");
        s
    }
}

/// Un caso listo para evaluar: qué mandar y cómo juzgar la respuesta.
///
/// El juicio llega como función para que el evaluador no tenga que conocer
/// cada verificador: el de protocolo vive en [`crate::protocolo`] y el de
/// programación en [`crate::programa`], y los dos leen la partición reservada.
pub struct CasoEval<'a> {
    pub id: String,
    pub clase: String,
    pub peticion: String,
    /// Devuelve `Ok(())` si la respuesta pasa, o el motivo si no.
    pub juez: &'a dyn Fn(&str) -> Result<(), String>,
    /// Si este caso **espera** una generación cuyo consumo haya que acreditar.
    ///
    /// Lo decide el caso, no el evaluador. Exigir `usage` a todo salía caro en
    /// la primera campaña real: un caso de error (400) y uno de streaming
    /// pasaban todas sus aserciones y se marcaban `sin-uso` porque el evaluador
    /// pedía un dato que esos casos nunca piden. Dos fallos que eran míos, no
    /// del modelo.
    pub exige_uso: bool,
}

/// Ejecuta la campaña. `semilla_base` se registra en cada intento para poder
/// repetirlo exactamente.
pub fn evaluar<C: ClienteModelo, R: Reloj>(
    cliente: &mut C,
    reloj: &R,
    casos: &[CasoEval<'_>],
    umbrales: Umbrales,
    plazo_por_peticion_ms: u64,
    semilla_base: u64,
    modelo: &str,
    banco: &str,
) -> Resultado<Informe> {
    if umbrales.repeticiones == 0 {
        return Err(Error::uso("las repeticiones no pueden ser cero"));
    }
    if plazo_por_peticion_ms == 0 {
        // C5: toda espera tiene tope. Un plazo infinito convierte una campaña
        // en algo que no termina y no se puede comparar.
        return Err(Error::uso("el plazo por petición no puede ser infinito"));
    }

    let mut intentos: Vec<Intento> = Vec::new();
    for caso in casos {
        for rep in 0..umbrales.repeticiones {
            let semilla = semilla_base
                .wrapping_add(rep as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let plazo = Plazo::en(reloj, plazo_por_peticion_ms);
            let mut intento = Intento {
                caso: caso.id.clone(),
                repeticion: rep,
                semilla,
                frio: rep == 0,
                desenlace: Desenlace::Error,
                ms_primer_byte: 0,
                ms_primer_token: 0,
                ms_total: 0,
                tokens_salida: None,
                detalle: String::new(),
                documento: String::new(),
            };
            match cliente.completar(&caso.peticion, plazo) {
                Err(Error::Plazo(m)) => {
                    intento.desenlace = Desenlace::Plazo;
                    intento.detalle = m;
                }
                Err(e) => {
                    intento.desenlace = Desenlace::Error;
                    intento.detalle = e.to_string();
                }
                Ok(r) => {
                    intento.documento = recortar(&r.documento, DOCUMENTO_MAX);
                    intento.ms_primer_byte = r.ms_primer_byte;
                    intento.ms_primer_token = r.ms_primer_token;
                    intento.ms_total = r.ms_total;
                    intento.tokens_salida = r.tokens_salida;
                    // El orden importa: primero se juzga el contenido, y solo
                    // una respuesta correcta puede quedarse sin acreditar por
                    // falta de `usage`. Así el informe distingue «contestó mal»
                    // de «contestó bien pero no sé lo que costó».
                    match (caso.juez)(&r.documento) {
                        Err(motivo) => {
                            intento.desenlace = Desenlace::Mal;
                            intento.detalle = motivo;
                        }
                        Ok(()) if caso.exige_uso && !r.tiene_uso() => {
                            intento.desenlace = Desenlace::SinUso;
                            intento.detalle =
                                String::from("la respuesta pasa pero el endpoint no dio usage");
                        }
                        Ok(()) => intento.desenlace = Desenlace::Bien,
                    }
                }
            }
            intentos.push(intento);
        }
    }

    let clases = resumir(casos, &intentos);
    let presupuesto = presupuestar(&intentos);
    let mut motivos = Vec::new();
    comprobar_umbral(&clases, "protocolo", umbrales.protocolo_minimo, &mut motivos);
    comprobar_umbral(
        &clases,
        "programacion",
        umbrales.programacion_minimo,
        &mut motivos,
    );

    Ok(Informe {
        schema_version: ESQUEMA,
        modelo: modelo.to_string(),
        banco: banco.to_string(),
        umbrales,
        clases,
        intentos,
        presupuesto,
        go: motivos.is_empty(),
        motivos,
    })
}

fn comprobar_umbral(
    clases: &[ResumenClase],
    nombre: &str,
    minimo: usize,
    motivos: &mut Vec<String>,
) {
    let Some(c) = clases.iter().find(|c| c.clase == nombre) else {
        // Una clase que el plan exige y que no se evaluó **no** es un aprobado
        // tácito: es un motivo de no-go.
        if minimo > 0 {
            motivos.push(format!(
                "no se evaluó ningún caso de «{nombre}» y el umbral pide {minimo}"
            ));
        }
        return;
    };
    if c.aprobados() < minimo {
        motivos.push(format!(
            "«{nombre}»: {} de {} casos sólidos, el umbral pide {minimo}{}",
            c.aprobados(),
            c.casos_totales,
            if c.casos_inestables > 0 {
                format!(" ({} inestable(s), que no cuentan)", c.casos_inestables)
            } else {
                String::new()
            }
        ));
    }
}

fn resumir(casos: &[CasoEval<'_>], intentos: &[Intento]) -> Vec<ResumenClase> {
    let mut por_clase: BTreeMap<String, ResumenClase> = BTreeMap::new();
    for caso in casos {
        let r = por_clase
            .entry(caso.clase.clone())
            .or_insert_with(|| ResumenClase {
                clase: caso.clase.clone(),
                casos_solidos: 0,
                casos_inestables: 0,
                casos_totales: 0,
                intentos_bien: 0,
                intentos_totales: 0,
            });
        r.casos_totales += 1;
        let mios: Vec<&Intento> = intentos.iter().filter(|i| i.caso == caso.id).collect();
        let bien = mios.iter().filter(|i| i.ok()).count();
        r.intentos_bien += bien;
        r.intentos_totales += mios.len();
        if !mios.is_empty() && bien == mios.len() {
            r.casos_solidos += 1;
        } else if bien > 0 {
            r.casos_inestables += 1;
        }
    }
    por_clase.into_values().collect()
}

fn presupuestar(intentos: &[Intento]) -> Presupuesto {
    // Solo los intentos que fueron bien: el coste de un fallo no es el coste de
    // una tarea, y meterlo baja la mediana justo cuando el modelo va peor.
    let buenos: Vec<&Intento> = intentos.iter().filter(|i| i.ok()).collect();
    if buenos.is_empty() {
        return Presupuesto::default();
    }
    let mut totales: Vec<u64> = buenos.iter().map(|i| i.ms_total).collect();
    let mut primeros: Vec<u64> = buenos.iter().map(|i| i.ms_primer_token).collect();
    totales.sort_unstable();
    primeros.sort_unstable();
    let maximo = *totales.last().unwrap_or(&0);
    Presupuesto {
        ms_primer_token_mediana: mediana(&primeros),
        ms_total_mediana: mediana(&totales),
        ms_total_maximo: maximo,
        // El doble del peor caso observado: holgura suficiente para no cortar
        // una petición sana, y un número que sale de la medida, no de un
        // redondeo bonito.
        timeout_sugerido_ms: maximo.saturating_mul(2).max(1_000),
        tokens_salida_maximo: buenos.iter().filter_map(|i| i.tokens_salida).max().unwrap_or(0),
    }
}

fn mediana(ordenados: &[u64]) -> u64 {
    if ordenados.is_empty() {
        return 0;
    }
    let n = ordenados.len();
    if n % 2 == 1 {
        ordenados[n / 2]
    } else {
        (ordenados[n / 2 - 1] + ordenados[n / 2]) / 2
    }
}

/// Recorta por **frontera de carácter**: cortar por bytes en medio de un
/// multibyte es lo que costó [T56].
fn recortar(s: &str, max: usize) -> String {
    if s.len() <= max {
        return String::from(s);
    }
    let mut corte = max;
    while corte > 0 && !s.is_char_boundary(corte) {
        corte -= 1;
    }
    let mut out = String::from(&s[..corte]);
    out.push_str("…[recortado]");
    out
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
    use crate::tiempo::RelojSimulado;
    use alloc::boxed::Box;

    /// Backend simulado: se le dicta qué contesta en cada llamada.
    enum Guion {
        Bien { usage: bool },
        Mal,
        Plazo,
        Error,
    }

    struct ClienteFalso {
        guion: Vec<Guion>,
        llamada: usize,
    }

    impl ClienteModelo for ClienteFalso {
        fn completar(&mut self, _peticion: &str, _plazo: Plazo) -> Resultado<Respuesta> {
            let g = self.guion.get(self.llamada).unwrap_or(&Guion::Error);
            self.llamada += 1;
            match g {
                Guion::Plazo => Err(Error::plazo("el endpoint no contestó")),
                Guion::Error => Err(Error::entorno("conexión rota")),
                Guion::Mal => Ok(Respuesta {
                    documento: String::from("{\"texto\":\"cualquier cosa\"}"),
                    ms_primer_byte: 10,
                    ms_primer_token: 20,
                    ms_total: 100,
                    tokens_entrada: Some(5),
                    tokens_salida: Some(7),
                    memoria_mb: None,
                }),
                Guion::Bien { usage } => Ok(Respuesta {
                    documento: String::from("{\"texto\":\"correcto\"}"),
                    ms_primer_byte: 10,
                    ms_primer_token: 20,
                    ms_total: 100,
                    tokens_entrada: if *usage { Some(5) } else { None },
                    tokens_salida: if *usage { Some(7) } else { None },
                    memoria_mb: None,
                }),
            }
        }
    }

    fn juez_correcto(doc: &str) -> Result<(), String> {
        if doc.contains("correcto") {
            Ok(())
        } else {
            Err(String::from("el documento no cumple las aserciones"))
        }
    }

    fn casos<'a>(ids: &[(&str, &str)], juez: &'a dyn Fn(&str) -> Result<(), String>) -> Vec<CasoEval<'a>> {
        ids.iter()
            .map(|(id, clase)| CasoEval {
                id: id.to_string(),
                clase: clase.to_string(),
                peticion: String::from("{}"),
                juez,
                exige_uso: true,
            })
            .collect()
    }

    fn corre(guion: Vec<Guion>, ids: &[(&str, &str)], umbrales: Umbrales) -> Informe {
        let r = RelojSimulado::nuevo(0);
        let mut c = ClienteFalso { guion, llamada: 0 };
        let j: Box<dyn Fn(&str) -> Result<(), String>> = Box::new(juez_correcto);
        let cs = casos(ids, &*j);
        evaluar(&mut c, &r, &cs, umbrales, 5_000, 42, "modelo-x", "banco-y").unwrap()
    }

    fn umbrales(protocolo: usize, programacion: usize, reps: u32) -> Umbrales {
        Umbrales {
            protocolo_minimo: protocolo,
            programacion_minimo: programacion,
            repeticiones: reps,
        }
    }

    #[test]
    fn todo_bien_es_go() {
        let g = (0..6).map(|_| Guion::Bien { usage: true }).collect();
        let i = corre(g, &[("Q01", "protocolo"), ("Q02", "protocolo")], umbrales(2, 0, 3));
        assert!(i.go, "{:?}", i.motivos);
        let c = i.clase("protocolo").unwrap();
        assert_eq!(c.casos_solidos, 2);
        assert_eq!(c.intentos_bien, 6);
        assert_eq!(c.intentos_totales, 6);
    }

    /// La regla que más veredictos infla si no se respeta: un caso que acierta
    /// dos de tres veces **no** es un caso aprobado.
    #[test]
    fn un_caso_inestable_no_cuenta_como_aprobado() {
        let g = alloc::vec![
            Guion::Bien { usage: true },
            Guion::Mal,
            Guion::Bien { usage: true },
        ];
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        let c = i.clase("protocolo").unwrap();
        assert_eq!(c.casos_solidos, 0);
        assert_eq!(c.casos_inestables, 1);
        assert_eq!(c.intentos_bien, 2);
        assert_eq!(c.intentos_totales, 3, "todos los intentos van al denominador");
        assert!(!i.go);
        assert!(i.motivos[0].contains("inestable"), "{:?}", i.motivos);
    }

    /// Una respuesta correcta sin `usage` no se puede acreditar.
    #[test]
    fn sin_usage_no_es_exito() {
        let g = (0..3).map(|_| Guion::Bien { usage: false }).collect();
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        assert!(!i.go);
        assert!(i.intentos.iter().all(|x| x.desenlace == Desenlace::SinUso));
        assert_eq!(i.clase("protocolo").unwrap().intentos_bien, 0);
    }

    /// El reverso: un caso que **no** pide `usage` —un error 400, un flujo
    /// SSE— no puede penalizarse por no traerlo. Lo decide el caso.
    #[test]
    fn un_caso_que_no_pide_usage_no_se_penaliza() {
        let r = RelojSimulado::nuevo(0);
        let mut c = ClienteFalso {
            guion: (0..3).map(|_| Guion::Bien { usage: false }).collect(),
            llamada: 0,
        };
        let j: Box<dyn Fn(&str) -> Result<(), String>> = Box::new(juez_correcto);
        let cs = alloc::vec![CasoEval {
            id: String::from("Q09"),
            clase: String::from("protocolo"),
            peticion: String::from("{}"),
            juez: &*j,
            exige_uso: false,
        }];
        let i = evaluar(&mut c, &r, &cs, umbrales(1, 0, 3), 5_000, 1, "m", "b").unwrap();
        assert!(i.go, "{:?}", i.motivos);
        assert!(i.intentos.iter().all(|x| x.desenlace == Desenlace::Bien));
    }

    #[test]
    fn el_plazo_y_el_error_se_distinguen() {
        let g = alloc::vec![Guion::Plazo, Guion::Error, Guion::Bien { usage: true }];
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        let estados: Vec<Desenlace> = i.intentos.iter().map(|x| x.desenlace).collect();
        assert_eq!(
            estados,
            alloc::vec![Desenlace::Plazo, Desenlace::Error, Desenlace::Bien]
        );
        assert!(!i.go);
    }

    /// Una salida errónea es «mal», no «error»: el endpoint funcionó.
    #[test]
    fn salida_erronea_es_mal_no_error() {
        let g = (0..3).map(|_| Guion::Mal).collect();
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        assert!(i.intentos.iter().all(|x| x.desenlace == Desenlace::Mal));
        assert!(!i.go);
    }

    /// Una clase que el plan exige y que no se evaluó no aprueba por omisión.
    #[test]
    fn una_clase_ausente_es_no_go() {
        let g = (0..3).map(|_| Guion::Bien { usage: true }).collect();
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 8, 3));
        assert!(!i.go);
        assert!(
            i.motivos.iter().any(|m| m.contains("programacion")),
            "{:?}",
            i.motivos
        );
    }

    #[test]
    fn el_primer_intento_es_frio() {
        let g = (0..3).map(|_| Guion::Bien { usage: true }).collect();
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        assert!(i.intentos[0].frio);
        assert!(!i.intentos[1].frio);
        assert!(!i.intentos[2].frio);
    }

    #[test]
    fn las_semillas_quedan_registradas_y_no_se_repiten() {
        let g = (0..3).map(|_| Guion::Bien { usage: true }).collect();
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        let s: Vec<u64> = i.intentos.iter().map(|x| x.semilla).collect();
        assert_eq!(s.len(), 3);
        assert_ne!(s[0], s[1]);
        assert_ne!(s[1], s[2]);
    }

    /// El presupuesto sale de los intentos **buenos**: el coste de un fallo no
    /// es el coste de una tarea.
    #[test]
    fn el_presupuesto_sale_de_lo_medido() {
        let g = (0..3).map(|_| Guion::Bien { usage: true }).collect();
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        assert_eq!(i.presupuesto.ms_total_mediana, 100);
        assert_eq!(i.presupuesto.ms_total_maximo, 100);
        assert_eq!(i.presupuesto.timeout_sugerido_ms, 1_000);
        assert_eq!(i.presupuesto.tokens_salida_maximo, 7);
    }

    #[test]
    fn sin_plazo_finito_no_hay_campana() {
        let r = RelojSimulado::nuevo(0);
        let mut c = ClienteFalso {
            guion: Vec::new(),
            llamada: 0,
        };
        let j: Box<dyn Fn(&str) -> Result<(), String>> = Box::new(juez_correcto);
        let cs = casos(&[("Q01", "protocolo")], &*j);
        let e = evaluar(&mut c, &r, &cs, Umbrales::default(), 0, 1, "m", "b").unwrap_err();
        assert!(format!("{e}").contains("infinito"), "{e}");
    }

    /// Sin el documento crudo, un fallo no se puede atribuir. Se guarda
    /// siempre que haya respuesta, pase o no.
    #[test]
    fn el_documento_crudo_se_guarda() {
        let g = alloc::vec![Guion::Mal, Guion::Bien { usage: true }, Guion::Plazo];
        let i = corre(g, &[("Q07", "protocolo")], umbrales(1, 0, 3));
        assert!(i.intentos[0].documento.contains("cualquier cosa"));
        assert!(i.intentos[1].documento.contains("correcto"));
        // Un plazo no trae documento: no hubo respuesta que guardar.
        assert!(i.intentos[2].documento.is_empty());
    }

    #[test]
    fn el_documento_se_recorta_sin_partir_un_caracter() {
        let largo = "☕".repeat(DOCUMENTO_MAX);
        let r = recortar(&largo, DOCUMENTO_MAX);
        assert!(r.len() <= DOCUMENTO_MAX + "…[recortado]".len());
        assert!(r.ends_with("…[recortado]"));
        // Lo que importa: sigue siendo UTF-8 válido, no un byte suelto.
        assert!(core::str::from_utf8(r.as_bytes()).is_ok());
    }

    #[test]
    fn el_informe_json_dice_el_veredicto_y_los_motivos() {
        let g = (0..3).map(|_| Guion::Mal).collect();
        let i = corre(g, &[("Q01", "protocolo")], umbrales(1, 0, 3));
        let j = i.json();
        assert!(j.contains("\"go\":false"), "{j}");
        assert!(j.contains("\"clase\":\"protocolo\""), "{j}");
        assert!(j.contains("\"motivos\":["), "{j}");
        assert!(j.contains("schema_version"), "{j}");
    }
}

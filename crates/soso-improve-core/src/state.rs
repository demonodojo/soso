//! Estado del coordinador: tareas, intentos y transiciones (T23).
//!
//! Esto es sólo el **formato y sus reglas**. No lanza builds ni llama a
//! OpenCode; eso llega en T24–T26. Lo que se fija aquí es lo que después no se
//! podrá cambiar sin romper estados ya escritos, así que conviene que las
//! decisiones incómodas estén tomadas desde el principio:
//!
//! - **Sólo el validador escribe `Aceptada`.** Un coordinador que puede
//!   aprobar su propio trabajo no está validando nada. La autoridad viaja en
//!   la transición, no en un comentario.
//! - **Lo que no se midió se dice, no se pone a cero.** Un `0` en «tokens
//!   usados» se lee como «no gastó nada», que es mentira cuando lo que pasa es
//!   que el endpoint no lo dijo. Por eso [`Medida::Desconocida`] lleva motivo.
//! - **En el estado van referencias, no logs.** Un estado que crece con cada
//!   intento acaba siendo imposible de escribir de forma durable.
//! - **Las comprobaciones son argv + cwd + plazo**, nunca una línea de shell:
//!   C5 lo exige para que un dato del modelo no se interpole en una orden.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::{Error, Resultado, ESQUEMA};

/// En qué punto está una tarea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Estado {
    Pendiente,
    Reproduciendo,
    Editando,
    Verificando,
    Aceptada,
    Rechazada,
    Bloqueada,
}

impl Estado {
    pub fn nombre(self) -> &'static str {
        match self {
            Estado::Pendiente => "pendiente",
            Estado::Reproduciendo => "reproduciendo",
            Estado::Editando => "editando",
            Estado::Verificando => "verificando",
            Estado::Aceptada => "aceptada",
            Estado::Rechazada => "rechazada",
            Estado::Bloqueada => "bloqueada",
        }
    }

    /// Estados de los que ya no se sale.
    pub fn terminal(self) -> bool {
        matches!(self, Estado::Aceptada | Estado::Rechazada)
    }
}

/// Quién pide la transición. No es decorativo: `Aceptada` sólo la puede
/// escribir el validador (C5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Autoridad {
    Coordinador,
    Validador,
}

/// Transiciones permitidas, con su autoridad.
///
/// Se devuelve `Err` con el motivo en vez de un booleano: quien la rechaza
/// tiene que poder decir por qué, y «transición inválida» a secas no ayuda a
/// nadie a las tres de la mañana.
pub fn puede_pasar(de: Estado, a: Estado, quien: Autoridad) -> Resultado<()> {
    if de.terminal() {
        return Err(Error::uso(format!(
            "{} es terminal: no se puede pasar a {}",
            de.nombre(),
            a.nombre()
        )));
    }
    if a == Estado::Aceptada && quien != Autoridad::Validador {
        return Err(Error::uso(
            "sólo el validador puede aceptar: el coordinador no aprueba su propio trabajo",
        ));
    }
    let permitido = match (de, a) {
        // Cualquier cosa puede bloquearse o rechazarse: un fallo de entorno o
        // una decisión humana no tienen que seguir el camino feliz.
        (_, Estado::Bloqueada) | (_, Estado::Rechazada) => true,
        (Estado::Pendiente, Estado::Reproduciendo) => true,
        (Estado::Reproduciendo, Estado::Editando) => true,
        (Estado::Editando, Estado::Verificando) => true,
        // Verificando puede volver a editar: un intento fallido no cierra la
        // tarea mientras queden intentos.
        (Estado::Verificando, Estado::Editando) => true,
        (Estado::Verificando, Estado::Aceptada) => true,
        // De bloqueada se puede volver cuando se resuelve el bloqueo.
        (Estado::Bloqueada, Estado::Pendiente) => true,
        _ => false,
    };
    if permitido {
        Ok(())
    } else {
        Err(Error::uso(format!(
            "transición no permitida: {} → {}",
            de.nombre(),
            a.nombre()
        )))
    }
}

/// Una medida que puede no existir.
///
/// El caso que esto evita: escribir `0` cuando el endpoint no dijo cuántos
/// tokens gastó. Un cero se suma, se promedia y se cree; un desconocido con
/// motivo no.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Medida {
    Valor(u64),
    /// Serializa como `{"desconocida": "motivo"}`.
    Desconocida { desconocida: String },
}

impl Medida {
    pub fn desconocida(motivo: impl ToString) -> Self {
        Medida::Desconocida {
            desconocida: motivo.to_string(),
        }
    }

    pub fn valor(&self) -> Option<u64> {
        match self {
            Medida::Valor(v) => Some(*v),
            Medida::Desconocida { .. } => None,
        }
    }
}

/// Una comprobación: argv y cwd explícitos, plazo finito. **Nunca** una línea
/// de shell (C5): interpolar un dato del modelo en una orden es cómo se
/// ejecuta lo que el modelo quiera.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comprobacion {
    pub argv: Vec<String>,
    pub cwd: String,
    pub timeout_s: u32,
}

/// Topes de una tarea. Los de C5 por defecto: 3 intentos, 30 herramientas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limites {
    pub intentos_max: u32,
    pub herramientas_max: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_max: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segundos_max: Option<u64>,
}

impl Default for Limites {
    fn default() -> Self {
        Limites {
            intentos_max: 3,
            herramientas_max: 30,
            tokens_max: None,
            segundos_max: None,
        }
    }
}

/// Qué se le pide a un candidato.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub schema_version: u32,
    pub id: String,
    /// Huella de la captura base (T01).
    pub base: String,
    pub problema: String,
    pub rutas_editables: Vec<String>,
    pub comprobaciones: Vec<Comprobacion>,
    pub limites: Limites,
    /// Hashes de las entradas, para saber si la tarea cambió bajo los pies.
    pub hashes_entrada: BTreeMap<String, String>,
}

impl TaskSpec {
    pub fn validar(&self) -> Resultado<()> {
        if self.schema_version != ESQUEMA {
            return Err(Error::formato(format!(
                "TaskSpec de esquema {}; esta herramienta habla {ESQUEMA}",
                self.schema_version
            )));
        }
        id_valido(&self.id)?;
        if self.base.len() != 64 {
            return Err(Error::formato("la base debe ser un sha256 hexadecimal"));
        }
        if self.rutas_editables.is_empty() {
            return Err(Error::uso(format!(
                "{}: sin rutas editables no hay nada que el candidato pueda tocar",
                self.id
            )));
        }
        for r in &self.rutas_editables {
            ruta_relativa(r)?;
        }
        if self.comprobaciones.is_empty() {
            return Err(Error::uso(format!(
                "{}: sin comprobaciones no se puede aceptar nada",
                self.id
            )));
        }
        for c in &self.comprobaciones {
            if c.argv.is_empty() {
                return Err(Error::uso("una comprobación sin argv no ejecuta nada"));
            }
            if c.timeout_s == 0 {
                return Err(Error::uso(format!(
                    "{}: plazo cero en {:?}; toda espera tiene tope (C5)",
                    self.id, c.argv
                )));
            }
        }
        if self.limites.intentos_max == 0 {
            return Err(Error::uso("cero intentos no es un límite, es no hacer nada"));
        }
        Ok(())
    }
}

/// Un intento concreto sobre una tarea.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub numero: u32,
    pub inicio_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fin_ms: Option<u64>,
    pub estado: Estado,
    pub herramientas: Medida,
    pub tokens: Medida,
    /// **Referencias** a los artefactos, no su contenido: un estado que crece
    /// con cada log no se puede escribir de forma durable.
    #[serde(default)]
    pub artefactos: Vec<String>,
}

/// El estado durable de una ejecución.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunManifest {
    pub schema_version: u32,
    /// Identifica la ejecución. **No** es un PID: un PID reciclado no
    /// identifica nada (C5).
    pub run_id: String,
    pub task_id: String,
    pub creado_ms: u64,
    pub estado: Estado,
    pub intentos: Vec<Attempt>,
}

impl RunManifest {
    pub fn nuevo(run_id: &str, task_id: &str, creado_ms: u64) -> Resultado<Self> {
        id_valido(run_id)?;
        id_valido(task_id)?;
        Ok(RunManifest {
            schema_version: ESQUEMA,
            run_id: run_id.to_string(),
            task_id: task_id.to_string(),
            creado_ms,
            estado: Estado::Pendiente,
            intentos: Vec::new(),
        })
    }

    pub fn validar(&self) -> Resultado<()> {
        if self.schema_version != ESQUEMA {
            return Err(Error::formato(format!(
                "RunManifest de esquema {}; esta herramienta habla {ESQUEMA}",
                self.schema_version
            )));
        }
        id_valido(&self.run_id)?;
        id_valido(&self.task_id)?;
        for (i, a) in self.intentos.iter().enumerate() {
            if a.numero as usize != i {
                return Err(Error::formato(format!(
                    "intento {} numerado como {}",
                    i, a.numero
                )));
            }
        }
        Ok(())
    }

    /// Cambia de estado si la transición está permitida para esa autoridad.
    pub fn transicion(&mut self, a: Estado, quien: Autoridad) -> Resultado<()> {
        puede_pasar(self.estado, a, quien)?;
        self.estado = a;
        Ok(())
    }

    /// Abre un intento nuevo, respetando el tope.
    pub fn abrir_intento(&mut self, limites: &Limites, ahora_ms: u64) -> Resultado<u32> {
        if self.intentos.len() as u32 >= limites.intentos_max {
            return Err(Error::uso(format!(
                "{}: agotados los {} intentos",
                self.run_id, limites.intentos_max
            )));
        }
        let numero = self.intentos.len() as u32;
        self.intentos.push(Attempt {
            numero,
            inicio_ms: ahora_ms,
            fin_ms: None,
            estado: Estado::Editando,
            herramientas: Medida::desconocida("el intento no ha terminado"),
            tokens: Medida::desconocida("el intento no ha terminado"),
            artefactos: Vec::new(),
        });
        Ok(numero)
    }

    /// Cierra el intento abierto con su desenlace y sus medidas.
    ///
    /// `herramientas` y `tokens` se pasan como [`Medida`] a propósito: quien
    /// cierra el intento tiene que decidir explícitamente si los midió. No hay
    /// una sobrecarga que acepte `u64` y ponga cero por defecto, porque esa
    /// comodidad es exactamente cómo aparece un cero inventado en un informe.
    pub fn cerrar_intento(
        &mut self,
        estado: Estado,
        herramientas: Medida,
        tokens: Medida,
        fin_ms: u64,
    ) -> Resultado<()> {
        let abierto = self
            .intentos
            .iter_mut()
            .rev()
            .find(|a| a.fin_ms.is_none())
            .ok_or_else(|| Error::uso("no hay ningún intento abierto que cerrar"))?;
        if fin_ms < abierto.inicio_ms {
            return Err(Error::uso(format!(
                "el intento {} acabaría antes de empezar ({fin_ms} < {})",
                abierto.numero, abierto.inicio_ms
            )));
        }
        abierto.estado = estado;
        abierto.herramientas = herramientas;
        abierto.tokens = tokens;
        abierto.fin_ms = Some(fin_ms);
        Ok(())
    }

    /// Añade la **referencia** de un artefacto al intento abierto.
    ///
    /// Se rechaza lo que parezca contenido en vez de una referencia: el estado
    /// se reescribe entero en cada publicación, y un log pegado aquí lo haría
    /// crecer hasta que la escritura durable deje de ser práctica.
    pub fn anotar_artefacto(&mut self, referencia: &str) -> Resultado<()> {
        if referencia.len() > REFERENCIA_MAX || referencia.contains('\n') {
            return Err(Error::uso(format!(
                "en el estado van referencias, no contenido ({} bytes)",
                referencia.len()
            )));
        }
        let abierto = self
            .intentos
            .iter_mut()
            .rev()
            .find(|a| a.fin_ms.is_none())
            .ok_or_else(|| Error::uso("no hay ningún intento abierto"))?;
        if abierto.artefactos.len() >= ARTEFACTOS_MAX {
            return Err(Error::uso(format!(
                "más de {ARTEFACTOS_MAX} artefactos en un intento: eso ya no es un índice"
            )));
        }
        abierto.artefactos.push(referencia.to_string());
        Ok(())
    }

    pub fn intento_abierto(&mut self) -> Option<&mut Attempt> {
        self.intentos.iter_mut().rev().find(|a| a.fin_ms.is_none())
    }
}

/// Tope de una referencia de artefacto: una ruta, no un log.
pub const REFERENCIA_MAX: usize = 256;
/// Tope de artefactos por intento.
pub const ARTEFACTOS_MAX: usize = 64;

fn id_valido(id: &str) -> Resultado<()> {
    if id.is_empty() || id.len() > 64 {
        return Err(Error::uso(format!("id de longitud inválida: {id:?}")));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        // Los ids acaban en nombres de fichero: un id con `/` o `..` sería una
        // ruta disfrazada.
        return Err(Error::uso(format!(
            "id con caracteres no permitidos: {id:?}"
        )));
    }
    Ok(())
}

fn ruta_relativa(r: &str) -> Resultado<()> {
    if r.is_empty() || r.starts_with('/') || r.split('/').any(|s| s == "..") {
        return Err(Error::uso(format!("ruta editable inválida: {r}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Persistencia
// ---------------------------------------------------------------------------

/// De dónde salen los identificadores de ejecución.
///
/// Es un trait y no un contador global porque el host y el guest los sacan de
/// sitios distintos, y porque una prueba tiene que poder forzar una colisión
/// sin esperar a que ocurra sola.
pub trait Ids {
    fn nuevo_run_id(&mut self) -> String;
}

/// Ids derivados del reloj: `r-<ms en hex>`.
///
/// No es un UUID y no pretende serlo: basta con que dos ejecuciones del mismo
/// milisegundo choquen **al publicar** en vez de mezclarse en silencio, que es
/// justo lo que hace [`publicar`](crate::durable::publicar) con su precondición.
pub struct IdsDelReloj<'a, R: crate::tiempo::Reloj> {
    pub reloj: &'a R,
    pub prefijo: &'a str,
}

impl<R: crate::tiempo::Reloj> Ids for IdsDelReloj<'_, R> {
    fn nuevo_run_id(&mut self) -> String {
        format!("{}-{:x}", self.prefijo, self.reloj.ahora_ms())
    }
}

/// Nombre de la referencia durable de una ejecución.
pub fn referencia(run_id: &str) -> String {
    format!("run-{run_id}")
}

/// Escribe el estado. `esperado` es el hash del estado que el llamante cree
/// que hay; `None` significa «esto es nuevo».
///
/// El estado **no** se sobrescribe: va por generaciones (T46), así que si esta
/// escritura falla la anterior sigue siendo legible. Es la diferencia entre
/// perder un paso y perder la ejecución entera.
pub fn guardar<D: crate::durable::Durable>(
    d: &mut D,
    dir: &str,
    manifiesto: &RunManifest,
    esperado: Option<&str>,
) -> Resultado<crate::durable::Generacion> {
    manifiesto.validar()?;
    let json = serde_json::to_vec(manifiesto)
        .map_err(|e| Error::formato(format!("no pude serializar el estado: {e}")))?;
    crate::durable::publicar(d, dir, &referencia(&manifiesto.run_id), &json, esperado)
}

/// Lee el estado vigente y su hash, para poder volver a escribirlo sin pisar a
/// nadie. `None` es «esa ejecución no existe», no un estado vacío.
pub fn cargar<D: crate::entorno::Archivos>(
    d: &D,
    dir: &str,
    run_id: &str,
) -> Resultado<Option<(RunManifest, String)>> {
    id_valido(run_id)?;
    let Some((_, datos)) = crate::durable::leer_vigente(d, dir, &referencia(run_id))? else {
        return Ok(None);
    };
    let m: RunManifest = serde_json::from_slice(&datos).map_err(|e| {
        // Un estado ilegible es un error de formato, no «no hay estado»:
        // confundirlos haría que una ejecución a medias pareciera nueva y se
        // repitieran sus efectos.
        Error::formato(format!("estado de {run_id} ilegible: {e}"))
    })?;
    m.validar()?;
    let hash = crate::sha256_hex(&datos);
    Ok(Some((m, hash)))
}

// ---------------------------------------------------------------------------
// Órdenes del coordinador
// ---------------------------------------------------------------------------
//
// Lo que sigue lo llaman los dos frontends tal cual. Está aquí y no en cada
// uno porque el criterio de «esto se puede hacer» no puede diferir entre el
// host y el guest: si difiere, una campaña dice cosas distintas según dónde
// corra y ya no acredita nada.

/// Nombre de la referencia durable donde vive el enunciado de la tarea.
pub fn referencia_spec(run_id: &str) -> String {
    format!("spec-{run_id}")
}

/// `tarea preparar`: deja el enunciado y el estado inicial escritos.
///
/// No hace la copia de trabajo ni exporta parche alguno: eso es T24. Lo que
/// esta orden garantiza es que, si el proceso se cae justo después, la
/// ejecución existe y se puede reanudar.
pub fn preparar<D: crate::durable::Durable>(
    d: &mut D,
    dir: &str,
    spec_json: &[u8],
    run_id: &str,
    ahora_ms: u64,
) -> Resultado<RunManifest> {
    let spec: TaskSpec = serde_json::from_slice(spec_json)
        .map_err(|e| Error::formato(format!("el enunciado no se pudo leer: {e}")))?;
    spec.validar()?;
    let manifiesto = RunManifest::nuevo(run_id, &spec.id, ahora_ms)?;

    // El enunciado primero: un estado que apunta a una tarea que no está
    // escrita es peor que no tener estado.
    let json = serde_json::to_vec(&spec)
        .map_err(|e| Error::formato(format!("no pude serializar el enunciado: {e}")))?;
    crate::durable::publicar(d, dir, &referencia_spec(run_id), &json, None)?;
    guardar(d, dir, &manifiesto, None)?;
    Ok(manifiesto)
}

/// Por dónde se quedó una ejecución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reanudacion {
    pub manifiesto: RunManifest,
    pub spec: TaskSpec,
    /// Hay un intento abierto: se cayó a mitad y **no** se puede repetir sin
    /// más, porque sus efectos ya existen (eso lo resuelve T27).
    pub intento_abierto: Option<u32>,
    pub intentos_restantes: u32,
}

/// `tarea reanudar`: dice dónde se quedó. **No continúa nada.**
///
/// La distinción importa: repetir los efectos de un intento a medias es
/// precisamente lo que [T27](../../docs/self-improvement/T27-reanudacion.md)
/// tiene que evitar. Esta orden informa; no actúa.
pub fn reanudar<D: crate::entorno::Archivos>(
    d: &D,
    dir: &str,
    run_id: &str,
) -> Resultado<Reanudacion> {
    let Some((manifiesto, _)) = cargar(d, dir, run_id)? else {
        return Err(Error::uso(format!("no hay ninguna ejecución «{run_id}»")));
    };
    let Some((_, crudo)) = crate::durable::leer_vigente(d, dir, &referencia_spec(run_id))? else {
        return Err(Error::uso(format!(
            "la ejecución «{run_id}» existe pero su enunciado no: no se puede reanudar a ciegas"
        )));
    };
    let spec: TaskSpec = serde_json::from_slice(&crudo)
        .map_err(|e| Error::formato(format!("enunciado de {run_id} ilegible: {e}")))?;
    let intento_abierto = manifiesto
        .intentos
        .iter()
        .rev()
        .find(|a| a.fin_ms.is_none())
        .map(|a| a.numero);
    let intentos_restantes = spec
        .limites
        .intentos_max
        .saturating_sub(manifiesto.intentos.len() as u32);
    Ok(Reanudacion {
        manifiesto,
        spec,
        intento_abierto,
        intentos_restantes,
    })
}

/// `tarea informe`: el estado en líneas legibles por la consola serie.
pub fn informe(r: &Reanudacion) -> String {
    let m = &r.manifiesto;
    let mut s = format!(
        "ejecución {} · tarea {} · estado {}\n",
        m.run_id,
        m.task_id,
        m.estado.nombre()
    );
    for a in &m.intentos {
        s.push_str(&format!(
            "  intento {} {:12} herramientas {} tokens {} artefactos {}\n",
            a.numero,
            a.estado.nombre(),
            medida_texto(&a.herramientas),
            medida_texto(&a.tokens),
            a.artefactos.len()
        ));
    }
    if let Some(n) = r.intento_abierto {
        s.push_str(&format!(
            "  intento {n} quedó abierto: sus efectos ya existen (reanudar sin repetirlos es T27)\n"
        ));
    }
    s.push_str(&format!(
        "  intentos restantes: {}\n",
        r.intentos_restantes
    ));
    s
}

/// Una medida desconocida se imprime como desconocida, con su motivo. Nunca
/// como `0`: un cero en un informe se suma y se cree.
fn medida_texto(m: &Medida) -> String {
    match m {
        Medida::Valor(v) => format!("{v}"),
        Medida::Desconocida { desconocida } => format!("? ({desconocida})"),
    }
}

//! Orden `evaluar`: campaña go/no-go contra un endpoint real (T14).
//!
//! La política —qué cuenta como éxito, cómo se cuentan los intentos, de dónde
//! salen los presupuestos— vive en `soso_improve_core::evaluacion` y es
//! portable. Aquí está solo lo que necesita `std`: hablar HTTP y leer el banco
//! del disco.

use std::time::Duration;

use soso_improve_core::caso::{self, CasoCargado};
use soso_improve_core::cli::Codigo;
use soso_improve_core::entorno::Archivos;
use soso_improve_core::evaluacion::{
    self, CasoEval, ClienteModelo, Respuesta, Umbrales,
};
use soso_improve_core::protocolo;
use soso_improve_core::tiempo::{Plazo, Reloj};
use soso_improve_core::{unir, Error, Resultado};

use crate::banco::{raiz_banco, raiz_reservada};
use crate::sistema::{Host, RelojHost};
use crate::Opciones;

/// Cliente HTTP sobre el endpoint de `soso-llm serve`.
///
/// Reutiliza el cliente de `soso-llm-api` que ya usa la campaña de T19: un
/// tercer cliente HTTP sería una tercera interpretación del mismo protocolo.
struct ClienteHttp {
    api: soso_llm_api::ApiClient,
    /// Nombre del modelo **en el catálogo del endpoint**.
    ///
    /// Las peticiones grabadas del banco traen el nombre con el que se
    /// grabaron, y el servidor compara por igualdad exacta: si no se sustituye,
    /// la campaña entera da 404 por un motivo que no tiene nada que ver con la
    /// calidad del modelo.
    catalogo: String,
    reloj: RelojHost,
    connect: Duration,
    read: Duration,
}

impl ClienteModelo for ClienteHttp {
    fn completar(&mut self, peticion: &str, _plazo: Plazo) -> Resultado<Respuesta> {
        let cuerpo = sustituir_modelo(peticion, &self.catalogo)?;
        let t0 = self.reloj.ahora_ms();
        let crudo = self
            .api
            .post_json_timeouts("/v1/chat/completions", &cuerpo, true, self.connect, self.read)
            .map_err(|e| {
                // El cliente devuelve texto; distinguir el vencimiento del
                // resto importa, porque en el informe son desenlaces distintos.
                if e.contains("timed out") || e.contains("temporarily unavailable") {
                    Error::plazo(e)
                } else {
                    Error::entorno(e)
                }
            })?;
        let ms = self.reloj.ahora_ms().saturating_sub(t0);
        let texto = String::from_utf8_lossy(&crudo).into_owned();
        let (estado, cabeceras, cuerpo_txt) = partir_http(&texto);
        let documento = documento_grabado(estado, &cabeceras, &cuerpo_txt);
        let (entrada, salida) = usage(&cuerpo_txt);
        Ok(Respuesta {
            documento,
            // Sin streaming no se puede separar el primer byte del resto: se
            // dice lo que hay, no se inventa un desglose.
            ms_primer_byte: ms,
            ms_primer_token: ms,
            ms_total: ms,
            tokens_entrada: entrada,
            tokens_salida: salida,
            memoria_mb: None,
        })
    }
}

/// Cambia `"model"` por el nombre del catálogo, dejando el resto intacto.
fn sustituir_modelo(peticion: &str, catalogo: &str) -> Resultado<String> {
    let mut v: serde_json::Value = serde_json::from_str(peticion).map_err(Error::formato)?;
    let Some(obj) = v.as_object_mut() else {
        return Err(Error::formato("la petición grabada no es un objeto JSON"));
    };
    obj.insert(
        String::from("model"),
        serde_json::Value::String(catalogo.to_string()),
    );
    serde_json::to_string(&v).map_err(Error::formato)
}

/// Parte una respuesta HTTP cruda en estado, cabeceras y cuerpo.
fn partir_http(texto: &str) -> (Option<i64>, String, String) {
    let (cabeza, cuerpo) = match texto.find("\r\n\r\n") {
        Some(i) => (&texto[..i], texto[i + 4..].to_string()),
        None => (texto, String::new()),
    };
    let estado = cabeza
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<i64>().ok());
    (estado, cabeza.to_lowercase(), cuerpo)
}

/// Arma el **documento grabado** que espera el verificador de protocolo.
///
/// Esto no es un detalle de formato: el verificador juzga `http_status` y
/// `cuerpo` (o `trozos_b64` para SSE), y pasarle el cuerpo pelado hace que
/// **todas** las aserciones de estado fallen a la vez. La primera campaña dio
/// 0/30 por esto, y el patrón —`estado HTTP None` en los diez casos— delataba
/// al adaptador, no al modelo.
fn documento_grabado(estado: Option<i64>, cabeceras: &str, cuerpo: &str) -> String {
    let estado_json = match estado {
        Some(c) => c.to_string(),
        None => String::from("null"),
    };
    // Un flujo SSE no es JSON: viaja en base64 y el verificador lo desmonta en
    // eventos. Intentar parsearlo como objeto es lo que daba «la respuesta no
    // es JSON» en los casos con `stream: true`.
    if cabeceras.contains("text/event-stream") || cuerpo.starts_with("data:") {
        return format!(
            "{{\"http_status\":{estado_json},\"trozos_b64\":[\"{}\"]}}",
            b64(cuerpo.as_bytes())
        );
    }
    let cuerpo_json = match serde_json::from_str::<serde_json::Value>(cuerpo) {
        Ok(v) => v.to_string(),
        // Un cuerpo que no es JSON se entrega como cadena: el verificador dirá
        // que no cumple, que es la verdad, en vez de reventar al parsear.
        Err(_) => serde_json::Value::String(cuerpo.to_string()).to_string(),
    };
    format!("{{\"http_status\":{estado_json},\"cuerpo\":{cuerpo_json}}}")
}

/// Base64 estándar. Son quince líneas y evita una dependencia nueva.
fn b64(datos: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for trozo in datos.chunks(3) {
        let b = [
            trozo[0],
            *trozo.get(1).unwrap_or(&0),
            *trozo.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if trozo.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if trozo.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn usage(cuerpo: &str) -> (Option<u32>, Option<u32>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(cuerpo) else {
        return (None, None);
    };
    let u = &v["usage"];
    let leer = |k: &str| u[k].as_u64().map(|n| n as u32);
    (leer("prompt_tokens"), leer("completion_tokens"))
}

/// Casos de protocolo del banco, con su petición grabada y su juez reservado.
fn casos_protocolo<'a>(
    banco: &str,
    reservado: &str,
    casos: &'a [CasoCargado],
    jueces: &'a mut Vec<Box<dyn Fn(&str) -> Result<(), String>>>,
) -> Resultado<Vec<(String, String, String)>> {
    let _ = jueces;
    let mut fuera = Vec::new();
    for c in casos.iter().filter(|c| c.caso.clase == "protocolo") {
        let peticion_rel = c
            .caso
            .entrada
            .archivos
            .iter()
            .find(|a| a.ends_with("peticion.json"))
            .ok_or_else(|| {
                Error::uso(format!("{}: no declara su peticion.json", c.caso.id))
            })?;
        let peticion = Host.leer(&unir(banco, peticion_rel))?;
        let peticion = String::from_utf8(peticion)
            .map_err(|_| Error::formato(format!("{}: petición no UTF-8", c.caso.id)))?;
        // `{relleno}` no es contenido: es un marcador que el lanzador tiene que
        // expandir, y así lo dice el enunciado del caso. Enviándolo literal la
        // petición **cabe**, el servidor genera y contesta 200 — correcto—, y
        // el informe lo apunta como fallo del servicio. Fue el caso Q10 de la
        // campaña del 23-sep.
        let peticion = expandir_relleno(banco, &c.caso, &peticion)?;
        let esperado_ruta = unir(reservado, &format!("{}/esperado.json", c.caso.id));
        let esperado = String::from_utf8(Host.leer(&esperado_ruta)?)
            .map_err(|_| Error::formato(format!("{}: esperado no UTF-8", c.caso.id)))?;
        fuera.push((c.caso.id.clone(), peticion, esperado));
    }
    Ok(fuera)
}

/// Cuántos bytes de prompt hacen falta para pasarse del contexto declarado.
///
/// **Quedarse corto sale caro.** Con cuatro bytes por token el relleno cupo
/// dentro del contexto, el servidor aceptó la petición y se puso a generar 512
/// tokens: doce minutos de CPU y un cliente que se rinde por plazo. El informe
/// lo apuntó como `plazo`, que no era. Por eso el factor es ahora **ocho**
/// bytes por token: pasarse de relleno sólo cuesta unos kilobytes de cuerpo
/// —el tope es 1 MiB—, mientras que quedarse corto cuesta una generación
/// entera y un diagnóstico equivocado.
///
/// La forma exacta sería tokenizar aquí, pero eso obliga a pasarle a la orden
/// el directorio del modelo además de su nombre de catálogo; se deja anotado
/// como mejora, no como deuda silenciosa.
fn bytes_para_rebasar(context_tokens: u32) -> usize {
    (context_tokens as usize).saturating_mul(8).saturating_add(16_384)
}

/// Sustituye `{relleno}` repitiendo el `relleno.txt` del caso.
fn expandir_relleno(
    banco: &str,
    caso: &soso_improve_core::caso::Caso,
    peticion: &str,
) -> Resultado<String> {
    if !peticion.contains("{relleno}") {
        return Ok(String::from(peticion));
    }
    let rel = caso
        .entrada
        .archivos
        .iter()
        .find(|a| a.ends_with("relleno.txt"))
        .ok_or_else(|| {
            Error::uso(format!(
                "{}: usa {{relleno}} pero no declara su relleno.txt",
                caso.id
            ))
        })?;
    let trozo = String::from_utf8(Host.leer(&unir(banco, rel))?)
        .map_err(|_| Error::formato(format!("{}: relleno no UTF-8", caso.id)))?;
    let trozo = trozo.trim_end_matches('\n');
    if trozo.is_empty() {
        return Err(Error::uso(format!("{}: el relleno está vacío", caso.id)));
    }
    let objetivo = bytes_para_rebasar(CONTEXTO_DECLARADO);
    let veces = objetivo.div_ceil(trozo.len()).max(1);
    let mut relleno = String::with_capacity(veces * (trozo.len() + 1));
    for _ in 0..veces {
        relleno.push_str(trozo);
        relleno.push(' ');
    }
    // Por JSON, no a mano: el relleno va dentro de una cadena y hay que
    // escaparlo bien.
    let como_json = serde_json::Value::String(relleno).to_string();
    let sin_comillas = &como_json[1..como_json.len() - 1];
    Ok(peticion.replace("{relleno}", sin_comillas))
}

/// Contexto que declara el perfil del modelo fijado (T03: `max_seq` 32768).
const CONTEXTO_DECLARADO: u32 = 32_768;

pub fn evaluar(opciones: &Opciones) -> Resultado<i32> {
    let banco = raiz_banco(opciones);
    let reservado = raiz_reservada(opciones, &banco);
    let puerto: u16 = opciones
        .uno("puerto")
        .unwrap_or("17299")
        .parse()
        .map_err(|_| Error::uso("--puerto inválido"))?;
    let token = opciones.uno("token").unwrap_or("").to_string();
    let catalogo = opciones.exigido("modelo")?.to_string();
    let repeticiones: u32 = opciones
        .uno("repeticiones")
        .unwrap_or("3")
        .parse()
        .map_err(|_| Error::uso("--repeticiones inválido"))?;
    let plazo_ms: u64 = opciones
        .uno("plazo-ms")
        .unwrap_or("600000")
        .parse()
        .map_err(|_| Error::uso("--plazo-ms inválido"))?;
    let semilla: u64 = opciones
        .uno("semilla")
        .unwrap_or("1")
        .parse()
        .map_err(|_| Error::uso("--semilla inválida"))?;

    // Repetir un solo caso cuesta minutos en vez de tres cuartos de hora, y es
    // lo que hace falta para atribuir un fallo concreto.
    let solo: Vec<String> = opciones
        .uno("caso")
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
        .unwrap_or_default();

    let cargados = caso::cargar(&Host, &banco)?;
    let mut jueces: Vec<Box<dyn Fn(&str) -> Result<(), String>>> = Vec::new();
    let mut crudos = casos_protocolo(&banco, &reservado, &cargados, &mut jueces)?;
    if !solo.is_empty() {
        crudos.retain(|(id, _, _)| solo.iter().any(|s| s == id));
        if crudos.is_empty() {
            return Err(Error::uso(format!(
                "ningún caso de protocolo coincide con --caso {}",
                solo.join(",")
            )));
        }
    }
    if crudos.is_empty() {
        return Err(Error::uso(format!("{banco} no tiene casos de protocolo")));
    }

    // Los jueces se construyen antes que los casos y viven más que ellos: cada
    // uno cierra sobre **su** `esperado.json` reservado, que el modelo no ve.
    let jueces: Vec<Box<dyn Fn(&str) -> Result<(), String>>> = crudos
        .iter()
        .map(|(id, _, esperado)| {
            let id = id.clone();
            let esperado = esperado.clone();
            let f: Box<dyn Fn(&str) -> Result<(), String>> = Box::new(move |doc: &str| {
                let esperado: protocolo::Esperado =
                    serde_json::from_str(&esperado).map_err(|e| format!("esperado ilegible: {e}"))?;
                let crudo: serde_json::Value = serde_json::from_str(doc)
                    .map_err(|e| format!("la respuesta no es JSON: {e}"))?;
                let informe = protocolo::verificar(&id, &esperado, &crudo)
                    .map_err(|e| format!("verificador: {e}"))?;
                if informe.estado == "ok" {
                    Ok(())
                } else {
                    let fallidas: Vec<String> = informe
                        .aserciones
                        .iter()
                        .filter(|a| !a.paso)
                        .map(|a| format!("{}: {}", a.tipo, a.motivo.clone().unwrap_or_default()))
                        .collect();
                    Err(format!(
                        "{}/{} aserciones; {}",
                        informe.pasadas,
                        informe.total,
                        fallidas.join("; ")
                    ))
                }
            });
            f
        })
        .collect();

    let casos: Vec<CasoEval> = crudos
        .iter()
        .zip(jueces.iter())
        .map(|((id, peticion, esperado), juez)| CasoEval {
            id: id.clone(),
            clase: String::from("protocolo"),
            peticion: peticion.clone(),
            juez: juez.as_ref(),
            // Lo pide el caso: sólo si sus aserciones reservadas hablan de
            // `usage`. Un 400 o un flujo SSE no tienen por qué traerlo.
            exige_uso: esperado.contains("usage_coherente"),
        })
        .collect();

    let reloj = RelojHost::default();
    let mut cliente = ClienteHttp {
        api: soso_llm_api::ApiClient::new(puerto, token, catalogo.clone()),
        catalogo: catalogo.clone(),
        reloj: RelojHost::default(),
        connect: Duration::from_secs(10),
        read: Duration::from_millis(plazo_ms),
    };

    let umbrales = Umbrales {
        // El umbral de protocolo es «todos los que haya»: el plan pide 10/10 y
        // el banco trae 10, pero si alguien recorta el banco el umbral tiene
        // que recortarse con él, no quedarse en un 10 que ya no existe.
        protocolo_minimo: casos.len(),
        // Las microtareas necesitan compilador; no se evalúan aquí y la ficha
        // lo dice. Poner 8 aquí daría un no-go por una clase que ni se intentó.
        programacion_minimo: 0,
        repeticiones,
    };

    crate::digo!(
        "evaluar: {} caso(s) de protocolo x{repeticiones} contra 127.0.0.1:{puerto} (modelo {catalogo})",
        casos.len()
    );
    let informe = evaluacion::evaluar(
        &mut cliente,
        &reloj,
        &casos,
        umbrales,
        plazo_ms,
        semilla,
        &catalogo,
        &banco,
    )?;

    // El documento crudo va al log: es lo que permite atribuir un fallo sin
    // volver a lanzar la campaña entera.
    for i in &informe.intentos {
        if !i.ok() && !i.documento.is_empty() {
            crate::digo!("  {} rep{} documento: {}", i.caso, i.repeticion, i.documento);
        }
    }
    for i in &informe.intentos {
        crate::digo!(
            "  {} rep{} {} {:>7} ms {}{}",
            i.caso,
            i.repeticion,
            if i.frio { "frío   " } else { "caliente" },
            i.ms_total,
            i.desenlace.nombre(),
            if i.detalle.is_empty() {
                String::new()
            } else {
                format!("  ({})", i.detalle)
            }
        );
    }
    for c in &informe.clases {
        crate::digo!(
            "{}: {} sólidos, {} inestables, de {} casos ({}/{} intentos)",
            c.clase, c.casos_solidos, c.casos_inestables, c.casos_totales,
            c.intentos_bien, c.intentos_totales
        );
    }
    crate::digo!(
        "presupuesto: mediana {} ms, máximo {} ms, timeout sugerido {} ms, tokens máx {}",
        informe.presupuesto.ms_total_mediana,
        informe.presupuesto.ms_total_maximo,
        informe.presupuesto.timeout_sugerido_ms,
        informe.presupuesto.tokens_salida_maximo
    );
    for m in &informe.motivos {
        crate::aviso!("motivo de no-go: {m}");
    }
    crate::digo!("veredicto: {}", if informe.go { "GO" } else { "NO-GO" });

    if let Some(salida) = opciones.uno("out").filter(|s| !s.is_empty()) {
        let mut host = Host;
        host.escribir(salida, informe.json().as_bytes(), 0o644)?;
        crate::digo!("informe en {salida}");
    }

    // Un no-go **no** es un fallo de la herramienta: la campaña se hizo y el
    // resultado es que no. Por eso sale con el código de verificación fallida,
    // no con el de error.
    Ok(if informe.go {
        Codigo::Exito.como_i32()
    } else {
        Codigo::Verificacion.como_i32()
    })
}

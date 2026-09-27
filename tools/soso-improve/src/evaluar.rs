//! Orden `evaluar`: campaña go/no-go contra un endpoint real (T14).
//!
//! La política —qué cuenta como éxito, cómo se cuentan los intentos, de dónde
//! salen los presupuestos— vive en `soso_improve_core::evaluacion` y es
//! portable. Aquí está solo lo que necesita `std`: hablar HTTP y leer el banco
//! del disco.

use std::time::Duration;

use soso_improve_core::caso::{self, Caso, CasoCargado};
use soso_improve_core::cli::Codigo;
use soso_improve_core::entorno::Archivos;
use soso_improve_core::evaluacion::{
    self, CasoEval, ClienteModelo, Respuesta, Umbrales,
};
use soso_improve_core::protocolo;
use soso_improve_core::tiempo::{Plazo, Reloj};
use soso_improve_core::{unir, Error, Resultado};
use soso_llm_api::MAX_BODY_BYTES;
use soso_llm_core::tokenizer::Tokenizer;

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
    contexto: u32,
    tokeniza: Option<&Tokenizer>,
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
        let peticion = expandir_relleno(banco, &c.caso, &peticion, contexto, tokeniza)?;
        let esperado_ruta = unir(reservado, &format!("{}/esperado.json", c.caso.id));
        let esperado = String::from_utf8(Host.leer(&esperado_ruta)?)
            .map_err(|_| Error::formato(format!("{}: esperado no UTF-8", c.caso.id)))?;
        fuera.push((c.caso.id.clone(), peticion, esperado));
    }
    Ok(fuera)
}

/// Bytes de relleno con la heurística de ocho bytes por token.
///
/// Sirve cuando no hay tokenizer. Con el 7B (`max_seq` 131072) esa cuenta
/// pasa de 1 MiB, que es el tope del cuerpo HTTP, y el servidor contestaría
/// 413 en vez de 422. En ese caso hay que tokenizar: el párrafo del banco
/// sale a unos 4,4 bytes por token y sí cabe.
fn bytes_para_rebasar(context_tokens: u32) -> usize {
    (context_tokens as usize).saturating_mul(8).saturating_add(16_384)
}

/// Sustituye `{relleno}` hasta pasarse del contexto del modelo.
fn expandir_relleno(
    banco: &str,
    caso: &Caso,
    peticion: &str,
    contexto: u32,
    tokeniza: Option<&Tokenizer>,
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
    let relleno = match tokeniza {
        Some(tok) => relleno_por_tokens(trozo, contexto, &|texto| tok.encode(texto).len())?,
        None => {
            let objetivo = bytes_para_rebasar(contexto);
            if objetivo > MAX_BODY_BYTES {
                return Err(Error::uso(format!(
                    "{}: el contexto de {contexto} tokens no cabe en el cuerpo HTTP \
                     con la heurística de 8 bytes/token; pasa --modelo-dir para tokenizar",
                    caso.id
                )));
            }
            repetir_trozo(trozo, objetivo.div_ceil(trozo.len()).max(1))
        }
    };
    let armada = incrustar_relleno(peticion, &relleno);
    if armada.len() > MAX_BODY_BYTES {
        return Err(Error::uso(format!(
            "{}: el relleno que rebasa {contexto} tokens ocupa {} bytes y el cuerpo admite {}",
            caso.id,
            armada.len(),
            MAX_BODY_BYTES
        )));
    }
    Ok(armada)
}

/// Repite el trozo hasta que su tokenización supere `contexto`.
///
/// El relleno solo, sin la plantilla ni la reserva de salida, ya tiene que
/// pasarse: lo que se añada después sólo hace el prompt más largo.
fn relleno_por_tokens(
    trozo: &str,
    contexto: u32,
    contar: &dyn Fn(&str) -> usize,
) -> Resultado<String> {
    let pieza_tokens = contar(trozo).max(1);
    let mut veces = (contexto as usize).div_ceil(pieza_tokens).saturating_add(1);
    for _ in 0..4 {
        let relleno = repetir_trozo(trozo, veces);
        let n = contar(&relleno);
        if n > contexto as usize {
            return Ok(relleno);
        }
        let faltan = (contexto as usize + 1 - n).div_ceil(pieza_tokens).max(1);
        veces = veces.saturating_add(faltan);
    }
    Err(Error::uso(format!(
        "no consigo un relleno de más de {contexto} tokens"
    )))
}

fn repetir_trozo(trozo: &str, veces: usize) -> String {
    let mut relleno = String::with_capacity(veces.saturating_mul(trozo.len().saturating_add(1)));
    for _ in 0..veces {
        relleno.push_str(trozo);
        relleno.push(' ');
    }
    relleno
}

/// Por JSON, no a mano: el relleno va dentro de una cadena y hay que escaparlo.
fn incrustar_relleno(peticion: &str, relleno: &str) -> String {
    let como_json = serde_json::Value::String(relleno.to_string()).to_string();
    let sin_comillas = &como_json[1..como_json.len() - 1];
    peticion.replace("{relleno}", sin_comillas)
}

/// Contexto del 3B cuando la campaña no dice otro. El 7B declara 131072.
const CONTEXTO_DECLARADO: u32 = 32_768;

fn cargar_tokenizer(dir: &str) -> Resultado<Tokenizer> {
    let ruta = unir(dir, "tokenizer.som");
    let datos = Host.leer(&ruta)?;
    Tokenizer::parse(&datos).map_err(|_| Error::formato(format!("{ruta}: tokenizer.som inválido")))
}

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

    let contexto: u32 = match opciones.uno("contexto") {
        Some(s) => s.parse().map_err(|_| Error::uso("--contexto inválido"))?,
        None => CONTEXTO_DECLARADO,
    };
    if contexto == 0 {
        return Err(Error::uso("--contexto tiene que ser mayor que cero"));
    }
    let tokeniza = match opciones.uno("modelo-dir").filter(|s| !s.is_empty()) {
        Some(dir) => Some(cargar_tokenizer(dir)?),
        None => None,
    };

    let cargados = caso::cargar(&Host, &banco)?;
    let mut jueces: Vec<Box<dyn Fn(&str) -> Result<(), String>>> = Vec::new();
    let mut crudos = casos_protocolo(
        &banco,
        &reservado,
        &cargados,
        &mut jueces,
        contexto,
        tokeniza.as_ref(),
    )?;
    let casos_en_banco = crudos.len();
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
        casos_en_banco,
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
    crate::digo!(
        "veredicto: {} ({})",
        if informe.go { "GO" } else { "NO-GO" },
        informe.cobertura.nombre()
    );

    if let Some(salida) = opciones.uno("out").filter(|s| !s.is_empty()) {
        let mut host = Host;
        host.escribir(salida, informe.json().as_bytes(), 0o644)?;
        crate::digo!("informe en {salida}");
    }

    // Un no-go medido sobre el banco entero no es un fallo de la herramienta.
    // Perder el transporte sí: no hubo campaña que verificar. Una tirada
    // filtrada sale con el código de verificación y `cobertura: filtrada`,
    // para que nadie la lea como el veredicto.
    Ok(match informe.cobertura {
        evaluacion::Cobertura::Interrumpida => Codigo::Error.como_i32(),
        evaluacion::Cobertura::Completa if informe.go => Codigo::Exito.como_i32(),
        evaluacion::Cobertura::Completa | evaluacion::Cobertura::Filtrada => {
            Codigo::Verificacion.como_i32()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{bytes_para_rebasar, incrustar_relleno, relleno_por_tokens};
    use soso_llm_api::MAX_BODY_BYTES;

    #[test]
    fn el_relleno_tokenizado_supera_el_contexto() {
        let trozo = "abcd efgh ijkl";
        // Cuatro bytes por token, como el párrafo medido del banco.
        let relleno = relleno_por_tokens(trozo, 100, &|s| s.len().div_ceil(4)).unwrap();
        assert!(relleno.len().div_ceil(4) > 100);
        assert!(relleno.len() < MAX_BODY_BYTES);
    }

    #[test]
    fn la_heuristica_de_128k_no_cabe_en_el_cuerpo() {
        assert!(bytes_para_rebasar(131_072) > MAX_BODY_BYTES);
        assert!(bytes_para_rebasar(32_768) < MAX_BODY_BYTES);
    }

    #[test]
    fn el_relleno_se_escapa_como_json() {
        let out = incrustar_relleno(r#"{"content":"{relleno}"}"#, "a\"b");
        assert_eq!(out, r#"{"content":"a\"b"}"#);
    }
}

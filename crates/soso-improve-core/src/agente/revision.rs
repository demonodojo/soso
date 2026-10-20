//! Revisión de lo hecho: el diff de los cambios del agente respecto al estado inicial de cada archivo que editó, más la lista de
//! requisitos de `TASK.md` («Qué se pide»). El agente puede pedirla con la herramienta `revisar`, y con `Config::revision` se le
//! impone una vez antes de dar el trabajo por terminado.
//!
//! Por qué: los fallos que quedan en el banco son casos límite del enunciado (NaN, empates, vacíos…) que el modelo olvida o
//! trata a medias; con el contexto plegado ni siquiera recuerda qué cambió. Aquí se le enseña su propio diff junto a los puntos
//! que debe cubrir, para que los contraste uno a uno antes de cerrar.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::entorno::Entorno;

const CONTEXTO: usize = 4;
const MAX_LINEAS_ARCHIVO: usize = 120;
const MAX_LINEAS_TOTAL: usize = 260;
const MAX_REQUISITOS: usize = 2500;
/// Tope de celdas de la tabla LCS (n·m); por encima, el tramo central se trata como borrado y reescrito.
const MAX_CELDAS: usize = 4_000_000;

/// Script de edición línea a línea entre `a` (antes) y `b` (después): `' '` igual, `'-'` quitada, `'+'` añadida.
pub fn diff_lineas<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(char, &'a str)> {
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (ma, mb) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut sal: Vec<(char, &str)> = a[..pre].iter().map(|l| (' ', *l)).collect();
    let (n, m) = (ma.len(), mb.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > MAX_CELDAS {
        sal.extend(ma.iter().map(|l| ('-', *l)));
        sal.extend(mb.iter().map(|l| ('+', *l)));
    } else {
        // lcs[i][j] = longitud de la subsecuencia común de ma[i..] y mb[j..]
        let w = m + 1;
        let mut lcs: Vec<u16> = alloc::vec![0; (n + 1) * w];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * w + j] = if ma[i] == mb[j] { lcs[(i + 1) * w + j + 1].saturating_add(1) } else { lcs[(i + 1) * w + j].max(lcs[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && ma[i] == mb[j] {
                sal.push((' ', ma[i]));
                i += 1;
                j += 1;
            } else if j < m && (i == n || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j]) {
                sal.push(('+', mb[j]));
                j += 1;
            } else {
                sal.push(('-', ma[i]));
                i += 1;
            }
        }
    }
    sal.extend(a[a.len() - suf..].iter().map(|l| (' ', *l)));
    sal
}

/// El script como hunks con `CONTEXTO` líneas alrededor; `None` si no hay ningún cambio.
fn hunks(script: &[(char, &str)], max_lineas: usize) -> Option<(String, usize)> {
    let cambios: Vec<usize> = script.iter().enumerate().filter(|(_, (c, _))| *c != ' ').map(|(i, _)| i).collect();
    if cambios.is_empty() {
        return None;
    }
    let mut mostrar = alloc::vec![false; script.len()];
    for &i in &cambios {
        for k in i.saturating_sub(CONTEXTO)..=(i + CONTEXTO).min(script.len() - 1) {
            mostrar[k] = true;
        }
    }
    // Número de línea (en el archivo nuevo) de cada posición del script.
    let mut nueva = alloc::vec![0usize; script.len()];
    let mut n = 0usize;
    for (k, (c, _)) in script.iter().enumerate() {
        if *c != '-' {
            n += 1;
        }
        nueva[k] = n;
    }
    let (mut t, mut lineas, mut previo) = (String::new(), 0usize, false);
    for (k, (c, l)) in script.iter().enumerate() {
        if !mostrar[k] {
            previo = false;
            continue;
        }
        if !previo {
            t.push_str(&format!("@@ cerca de la línea {} @@\n", nueva[k].max(1)));
            previo = true;
        }
        if lineas >= max_lineas {
            t.push_str("… (diff recortado)\n");
            return Some((t, lineas));
        }
        let l = if l.chars().count() > 160 { format!("{}…", l.chars().take(160).collect::<String>()) } else { (*l).to_string() };
        t.push_str(&format!("{c} {l}\n"));
        lineas += 1;
    }
    Some((t, lineas))
}

/// La sección «Qué se pide» de `TASK.md` (hasta el siguiente `## `), o `None` si no hay `TASK.md` o no tiene esa sección.
pub fn requisitos<E: Entorno>(e: &mut E, raiz: &str) -> Option<String> {
    let datos = e.leer(&format!("{}/TASK.md", raiz.trim_end_matches('/'))).ok()?;
    let texto = String::from_utf8(datos).ok()?;
    let mut dentro = false;
    let mut sal = String::new();
    for l in texto.lines() {
        if l.starts_with("## ") {
            if dentro {
                break;
            }
            dentro = l.to_lowercase().contains("qué se pide") || l.to_lowercase().contains("que se pide");
            continue;
        }
        if dentro {
            sal.push_str(l);
            sal.push('\n');
        }
    }
    let sal = sal.trim().to_string();
    if sal.is_empty() {
        return None;
    }
    Some(if sal.len() > MAX_REQUISITOS { format!("{}…", sal.chars().take(MAX_REQUISITOS).collect::<String>()) } else { sal })
}

/// El diff de lo editado y los requisitos de la tarea (sin instrucciones): lo que ve un revisor independiente.
pub fn contenido<E: Entorno>(e: &mut E, raiz: &str, originales: &BTreeMap<String, Option<String>>) -> String {
    let mut t = String::from("## Tus cambios respecto al inicio\n");
    let (mut total, mut hay) = (0usize, false);
    for (abs, antes) in originales {
        let rel = abs.strip_prefix(raiz.trim_end_matches('/')).unwrap_or(abs).trim_start_matches('/');
        let ahora = e.leer(abs).ok().and_then(|d| String::from_utf8(d).ok());
        let a: Vec<&str> = antes.as_deref().map(|s| s.lines().collect()).unwrap_or_default();
        let ahora_s = ahora.clone().unwrap_or_default();
        let b: Vec<&str> = ahora_s.lines().collect();
        let script = diff_lineas(&a, &b);
        match hunks(&script, MAX_LINEAS_ARCHIVO.min(MAX_LINEAS_TOTAL.saturating_sub(total))) {
            Some((h, n)) => {
                hay = true;
                total += n;
                let estado = match (antes.is_some(), ahora.is_some()) {
                    (false, true) => " (archivo nuevo)",
                    (true, false) => " (borrado)",
                    _ => "",
                };
                t.push_str(&format!("--- {rel}{estado}\n{h}"));
            }
            None => t.push_str(&format!("--- {rel}: sin cambios netos (lo editaste y quedó igual que al principio)\n")),
        }
    }
    if originales.is_empty() {
        t.push_str("Todavía no has editado ningún archivo.\n");
    } else if !hay {
        t.push_str("Ningún archivo ha cambiado en neto.\n");
    }
    t.push_str("\n## Lo que pide la tarea\n");
    match requisitos(e, raiz) {
        Some(r) => {
            t.push_str(&r);
            t.push('\n');
        }
        None => t.push_str("(no he encontrado la sección «Qué se pide» en TASK.md: vuelve a leerlo con `read`.)\n"),
    }
    t
}

/// Texto de la revisión para el propio agente: el diff, los requisitos y qué hacer con ambos.
pub fn texto<E: Entorno>(e: &mut E, raiz: &str, originales: &BTreeMap<String, Option<String>>) -> String {
    let mut t = contenido(e, raiz, originales);
    t.push_str(
        "\n## Qué haces ahora\nRecorre la lista punto por punto y, para cada uno, mira tu diff: ¿lo cubre de verdad, incluidos los casos límite (valores raros, \
         empates, vacíos, infinitos, errores)? ¿Hay algo que no hayas tocado o que hayas resuelto a medias? Si falta algo, corrígelo ahora y vuelve a \
         ejecutar las comprobaciones; si todo está cubierto, termina con un informe breve de qué cambiaste y por qué. No repitas cambios que ya están.",
    );
    t
}

/// Instrucción del revisor independiente (una petición aparte, sin la conversación del agente: así no hereda su sesgo de confirmación;
/// en la campaña 33 el agente se revisaba a sí mismo y siempre «confirmaba que todo estaba cubierto»).
pub const REVISOR: &str = "Eres un revisor de código estricto e independiente. Recibes los requisitos de una tarea y el diff de los cambios que ha hecho otro programador. NO conoces su conversación ni ejecutas código. Tu trabajo es encontrar fallos: recorre cada requisito y, para cada uno, piensa qué entradas límite podrían romperlo (NaN, infinitos, vacío, empates, valores justo en el borde de un rango, errores) y comprueba en el diff si el código las trata como pide el requisito. Responde SOLO con una lista numerada de como máximo 5 fallos probables; cada uno con: el requisito, una entrada concreta y qué devolvería el código del diff frente a lo que exige el requisito. No elogies ni resumas. Si de verdad no ves ningún fallo, responde exactamente: SIN FALLOS";

/// Petición (formato de la API, sin herramientas) para el revisor independiente.
pub fn peticion_revisor(modelo: &str, contenido: &str, max_tokens: u32) -> String {
    serde_json::json!({
        "model": modelo,
        "messages": [{"role": "system", "content": REVISOR}, {"role": "user", "content": contenido}],
        "temperature": 0.0, "max_tokens": max_tokens, "stream": false
    })
    .to_string()
}

/// La crítica del revisor, o `None` si no encontró fallos (o la respuesta no se puede leer).
pub fn critica(cuerpo: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(cuerpo).ok()?;
    let t = v.pointer("/choices/0/message/content")?.as_str()?.trim().to_string();
    if t.is_empty() || t.to_uppercase().starts_with("SIN FALLOS") {
        return None;
    }
    Some(t)
}

/// Para cada archivo editado, los números de línea (1-based, del archivo ACTUAL) que el agente ha añadido o cambiado respecto al original.
pub fn lineas_anadidas<E: Entorno>(e: &mut E, raiz: &str, originales: &BTreeMap<String, Option<String>>) -> BTreeMap<String, alloc::collections::BTreeSet<usize>> {
    let mut sal: BTreeMap<String, alloc::collections::BTreeSet<usize>> = BTreeMap::new();
    for (abs, antes) in originales {
        let rel = abs.strip_prefix(raiz.trim_end_matches('/')).unwrap_or(abs).trim_start_matches('/').to_string();
        let Some(ahora) = e.leer(abs).ok().and_then(|d| String::from_utf8(d).ok()) else { continue };
        let a: Vec<&str> = antes.as_deref().map(|s| s.lines().collect()).unwrap_or_default();
        let b: Vec<&str> = ahora.lines().collect();
        let mut n = 0usize;
        let mut set = alloc::collections::BTreeSet::new();
        for (c, _) in diff_lineas(&a, &b) {
            match c {
                '-' => {}
                '+' => {
                    n += 1;
                    set.insert(n);
                }
                _ => n += 1,
            }
        }
        if !set.is_empty() {
            sal.insert(rel, set);
        }
    }
    sal
}

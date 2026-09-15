//! Órdenes de verificación: `programa`, `protocolo` y `repo`.

use soso_improve_core::caso;
use soso_improve_core::entorno::Archivos;
use soso_improve_core::{programa, protocolo, repo, unir, Error, Resultado};

use crate::banco::{cargar, raiz_banco, raiz_reservada};
use crate::git;
use crate::sistema::{Host, Temporal};
use crate::Opciones;

fn escribir_informe<T: serde::Serialize>(opciones: &Opciones, informe: &T) -> Resultado<()> {
    if let Some(ruta) = opciones.uno("json").filter(|s| !s.is_empty()) {
        let mut texto = serde_json::to_string_pretty(informe).map_err(Error::formato)?;
        texto.push('\n');
        let mut host = Host;
        host.escribir(ruta, texto.as_bytes(), 0o644)?;
    }
    Ok(())
}

pub fn despachar(sub: &str, opciones: &Opciones) -> Resultado<i32> {
    match sub {
        "programa" => verificar_programa(opciones),
        "protocolo" => verificar_protocolo(opciones),
        "repo" => verificar_repo(opciones),
        otro => Err(Error::uso(format!(
            "verificar: subcomando desconocido {otro}"
        ))),
    }
}

fn verificar_programa(opciones: &Opciones) -> Resultado<i32> {
    let banco = raiz_banco(opciones);
    let reservado = raiz_reservada(opciones, &banco);
    let id = opciones.exigido("caso")?.to_string();
    let candidato = opciones.exigido("candidato")?.to_string();

    let casos = cargar(&banco)?;
    let c = caso::por_id(&casos, &id)?;
    if c.clase != "programacion" {
        return Err(Error::uso(format!("{id} no es un caso de programación")));
    }
    if !Host.existe(&candidato) {
        crate::aviso!("{id}: ERROR — no existe el candidato {candidato}");
        return Ok(1);
    }
    let vectores = programa::leer_vectores(&Host, &unir(&reservado, &format!("{id}/vectores.json")))?;
    let fuente = Host.leer(&candidato)?;

    let trabajo = Temporal::nuevo(&format!("banco-{id}"))?;
    let compilador = |fuente: &str, destino: &str| -> Vec<String> {
        vec![
            "rustc".to_string(),
            "--edition".to_string(),
            "2021".to_string(),
            "-O".to_string(),
            "-o".to_string(),
            destino.to_string(),
            fuente.to_string(),
        ]
    };
    let mut host = Host;
    let informe = programa::verificar(
        &id,
        &vectores,
        &fuente,
        &mut host,
        &Host,
        &trabajo.ruta(),
        &compilador,
        30,
    )?;
    escribir_informe(opciones, &informe)?;

    if informe.estado == "error" {
        crate::aviso!(
            "{id}: ERROR — {}",
            informe.motivo.as_deref().unwrap_or("desconocido")
        );
        if let Some(c) = &informe.compilacion {
            crate::aviso!("{c}");
        }
        return Ok(1);
    }
    crate::digo!(
        "{id}: {} ({}/{} vectores)",
        informe.estado, informe.pasados, informe.total
    );
    for v in informe.vectores.iter().filter(|v| !v.paso) {
        crate::aviso!(
            "  vector «{}»: esperado {:?} exit {}; obtenido {:?} exit {:?}{}",
            v.vector,
            v.esperado,
            v.exit_esperado,
            v.obtenido,
            v.exit,
            v.motivo
                .as_ref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default()
        );
    }
    Ok(informe.codigo())
}

fn verificar_protocolo(opciones: &Opciones) -> Resultado<i32> {
    let banco = raiz_banco(opciones);
    let reservado = raiz_reservada(opciones, &banco);
    let id = opciones.exigido("caso")?.to_string();
    let respuesta = opciones.exigido("respuesta")?.to_string();

    let casos = cargar(&banco)?;
    let c = caso::por_id(&casos, &id)?;
    if c.clase != "protocolo" {
        return Err(Error::uso(format!("{id} no es un caso de protocolo")));
    }
    if !Host.existe(&respuesta) {
        crate::aviso!("{id}: ERROR — no existe el documento {respuesta}");
        return Ok(1);
    }
    let esperado: protocolo::Esperado = serde_json::from_slice(
        &Host.leer(&unir(&reservado, &format!("{id}/esperado.json")))?,
    )
    .map_err(Error::formato)?;
    let crudo: serde_json::Value = match serde_json::from_slice(&Host.leer(&respuesta)?) {
        Ok(v) => v,
        Err(e) => {
            crate::aviso!("{id}: ERROR — documento ilegible: {e}");
            return Ok(1);
        }
    };

    let informe = protocolo::verificar(&id, &esperado, &crudo)?;
    escribir_informe(opciones, &informe)?;
    crate::digo!(
        "{id}: {} ({}/{} aserciones)",
        informe.estado, informe.pasadas, informe.total
    );
    for a in informe.aserciones.iter().filter(|a| !a.paso) {
        crate::aviso!("  {}: {}", a.tipo, a.motivo.as_deref().unwrap_or(""));
    }
    Ok(if informe.estado == "ok" { 0 } else { 2 })
}

fn verificar_repo(opciones: &Opciones) -> Resultado<i32> {
    let banco = raiz_banco(opciones);
    let reservado = raiz_reservada(opciones, &banco);
    let id = opciones.exigido("caso")?.to_string();
    // Ruta absoluta: los comandos se lanzan con el árbol como cwd, y una ruta
    // relativa pasada a `git -C` se resolvería otra vez desde ahí.
    let arbol = opciones.exigido("arbol")?;
    let arbol = std::fs::canonicalize(arbol)
        .map_err(|e| Error::uso(format!("{arbol}: {e}")))?
        .to_string_lossy()
        .into_owned();
    let con_referencia = opciones.bandera("con-referencia");
    let timeout: u32 = opciones
        .uno("timeout")
        .and_then(|t| t.parse().ok())
        .unwrap_or(900);

    let casos = cargar(&banco)?;
    let c = caso::por_id(&casos, &id)?.clone();
    if c.clase != "repo" {
        return Err(Error::uso(format!("{id} no es un caso de repo")));
    }
    // Git solo hace falta para aplicar la solución de referencia. Medir un
    // árbol tal cual no lo necesita, y así un árbol reconstruido sin git
    // —el caso de soso— también se puede medir.
    if con_referencia && !Host.existe(&unir(&arbol, ".git")) {
        return Err(Error::uso(format!(
            "{arbol} no es un árbol git: --con-referencia necesita aplicar un parche"
        )));
    }
    let prueba = Host.leer(&unir(&reservado, &format!("{id}/aceptacion.rs")))?;
    let parche = if con_referencia {
        let ruta = unir(&reservado, &format!("{id}/referencia.patch"));
        if !Host.existe(&ruta) {
            return Err(Error::uso(format!("no hay parche de referencia en {ruta}")));
        }
        Some(std::fs::canonicalize(&ruta)
            .map_err(|e| Error::uso(format!("{ruta}: {e}")))?
            .to_string_lossy()
            .into_owned())
    } else {
        None
    };

    let aplicar = |arbol: &str, parche: &str, revertir: bool| git::aplicar_parche(arbol, parche, revertir);
    let mut host = Host;
    let informe = repo::verificar(
        &c,
        &arbol,
        &mut host,
        &Host,
        &prueba,
        parche.as_deref(),
        &aplicar,
        timeout,
    )?;
    escribir_informe(opciones, &informe)?;

    if informe.estado == "error" {
        crate::aviso!(
            "{id}: ERROR — {}",
            informe.motivo.as_deref().unwrap_or("desconocido")
        );
        return Ok(1);
    }
    let marca = if informe.con_referencia {
        "con referencia"
    } else {
        "tal cual"
    };
    crate::digo!(
        "{id} ({marca}): {} (exit {:?})",
        informe.estado, informe.exit_code
    );
    if informe.estado != "ok" {
        let desde = informe.salida.len().saturating_sub(2000);
        crate::aviso!("{}", &informe.salida[desde..]);
    }
    Ok(informe.codigo())
}

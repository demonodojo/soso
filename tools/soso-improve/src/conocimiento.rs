//! `conocimiento` — notas verificadas entre tareas (T28). La lógica vive en `soso_improve_core::knowledge`.

use soso_improve_core::captura::{self, ArchivoBase};
use soso_improve_core::cli::Codigo;
use soso_improve_core::knowledge::{self, Certeza, Evidencia, Reservado};
use soso_improve_core::sha256_hex;
use soso_improve_core::validate;
use soso_improve_core::Error;

use crate::sistema::Host;
use crate::Opciones;

fn lista(op: &Opciones, clave: &str) -> Vec<String> {
    op.uno(clave).filter(|s| !s.is_empty()).map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default()
}

fn base_de(captura: &str) -> Result<(Vec<ArchivoBase>, String), Error> {
    let man = captura::leer_manifiesto(&Host, captura)?;
    let h = captura::huella(&man.inventario);
    Ok((man.inventario, h))
}

pub fn despachar(sub: &str, op: &Opciones) -> Result<i32, Error> {
    let dir = op.exigido("notas")?.to_string();
    match sub {
        "proponer" => {
            let (inv, huella) = base_de(op.exigido("captura")?)?;
            let mut evid = Vec::new();
            for f in op.varios("evidencia").into_iter().filter(|f| !f.is_empty()) {
                let datos = std::fs::read(&f).map_err(|e| Error::uso(format!("evidencia {f}: {e}")))?;
                evid.push(Evidencia { ruta: f, sha256: sha256_hex(&datos) });
            }
            let n = knowledge::candidata(
                op.exigido("problema")?,
                op.exigido("solucion")?,
                &lista(op, "rutas"),
                &lista(op, "pruebas"),
                &inv,
                &huella,
                op.uno("paquete-hash").unwrap_or(""),
                evid,
                op.bandera("fallido"),
            );
            let mut notas = knowledge::cargar(&Host, &dir)?;
            let id = n.id.clone();
            let nueva = knowledge::añadir(&mut notas, n);
            knowledge::guardar(&mut Host, &dir, &notas)?;
            println!("conocimiento: nota {id} {}", if nueva { "propuesta (hipótesis o limitación)" } else { "ya existía" });
            Ok(Codigo::Exito.como_i32())
        }
        "confirmar" => {
            let id = op.exigido("id")?;
            let mut notas = knowledge::cargar(&Host, &dir)?;
            let n = notas.iter_mut().find(|n| n.id == id).ok_or_else(|| Error::uso(format!("no hay ninguna nota {id}")))?;
            let vig = validate::base_vigente(&Host, op.exigido("referencias")?, op.exigido("campana")?)?;
            knowledge::confirmar(n, &Host, vig.as_ref().map(|(b, _)| b))?;
            knowledge::guardar(&mut Host, &dir, &notas)?;
            println!("conocimiento: nota {id} confirmada como hecho");
            Ok(Codigo::Exito.como_i32())
        }
        "invalidar" => {
            let (inv, huella) = base_de(op.exigido("captura")?)?;
            let mut notas = knowledge::cargar(&Host, &dir)?;
            let n = knowledge::invalidar_por_base(&mut notas, &inv, &huella);
            knowledge::guardar(&mut Host, &dir, &notas)?;
            println!("conocimiento: {n} nota(s) desactualizada(s) por el cambio de base");
            Ok(Codigo::Exito.como_i32())
        }
        "recuperar" => {
            let r = soso_improve_core::state::reanudar(&Host, op.exigido("estado")?, op.exigido("run")?)?;
            let notas = knowledge::cargar(&Host, &dir)?;
            let presupuesto = op.uno("presupuesto").and_then(|p| p.parse().ok()).unwrap_or(4000usize);
            let res = knowledge::recuperar(&notas, &r.spec, &Reservado { prefijos: lista(op, "reservado-prefijos") }, presupuesto);
            for s in &res.notas {
                println!("{}\n", s.texto);
            }
            println!(
                "conocimiento: {} nota(s) entregada(s) de {} guardadas; {} fuera de presupuesto; {} excluida(s) por reservadas o secretas",
                res.notas.len(),
                notas.len(),
                res.fuera_de_presupuesto,
                res.excluidas.len()
            );
            let _ = Certeza::Hecho;
            Ok(Codigo::Exito.como_i32())
        }
        otro => Err(Error::uso(format!("conocimiento necesita proponer|confirmar|invalidar|recuperar, no «{otro}»"))),
    }
}

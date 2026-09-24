//! `tarea` — las órdenes del coordinador (T23).
//!
//! Aquí sólo está la envoltura del host: leer y escribir archivos de verdad.
//! El formato, las transiciones y quién puede aceptar viven en
//! `soso_improve_core::state`, porque el guest tiene que aplicar exactamente el
//! mismo criterio. Si esa política se duplicara, una campaña diría cosas
//! distintas según dónde corriera.
//!
//! Lo que **no** está: ejecutar al candidato (T25) y validarlo (T26). Esas dos
//! órdenes existen en el CLI compartido y se rechazan con su ficha; no se
//! simula que hicieron algo.

use std::time::{SystemTime, UNIX_EPOCH};

use soso_improve_core::cli::Codigo;
use soso_improve_core::state::{self, Ids};
use soso_improve_core::workspace;
use soso_improve_core::Error;

use crate::sistema::Host;
use crate::Opciones;

/// Ids del host. No se usa [`RelojHost`](crate::sistema::RelojHost) porque es
/// monotónico **desde el arranque del proceso**: dos invocaciones seguidas
/// darían el mismo id, y el estado de una pisaría al de la otra.
struct IdsHost;

impl Ids for IdsHost {
    fn nuevo_run_id(&mut self) -> String {
        let ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("run-{ns:x}")
    }
}

fn ahora_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn despachar(sub: &str, op: &Opciones) -> Result<i32, Error> {
    match sub {
        "preparar" => preparar(op),
        "copiar" => copiar(op),
        "exportar" => exportar(op),
        "reanudar" => ver(op, true),
        "informe" => ver(op, false),
        // `capacidades().exigir` ya rechazó `ejecutar` y `validar` con su
        // ficha. Llegar aquí sería declararlas y no tenerlas.
        otro => Err(Error::uso(format!(
            "tarea necesita preparar|copiar|exportar|reanudar|informe, no «{otro}»"
        ))),
    }
}

fn preparar(op: &Opciones) -> Result<i32, Error> {
    let dir = op.exigido("estado")?;
    let spec = op.exigido("spec")?;
    let json = std::fs::read(spec)
        .map_err(|e| Error::entorno(format!("no pude leer el enunciado {spec}: {e}")))?;
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::entorno(format!("no pude crear {dir}: {e}")))?;

    let run_id = match op.uno("run") {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => IdsHost.nuevo_run_id(),
    };
    let m = state::preparar(&mut Host, dir, &json, &run_id, ahora_ms())?;
    println!("tarea: preparada {} para {}", m.run_id, m.task_id);
    println!("  estado en {dir}");
    Ok(Codigo::Exito.como_i32())
}

fn ver(op: &Opciones, reanudar: bool) -> Result<i32, Error> {
    let dir = op.exigido("estado")?;
    let run = op.exigido("run")?;
    let r = state::reanudar(&Host, dir, run)?;
    print!("{}", state::informe(&r));
    if reanudar {
        // Informar de por dónde va no es continuar, y la orden no puede dar a
        // entender lo contrario: repetir los efectos de un intento a medias es
        // justo lo que T27 tiene que evitar.
        println!("tarea: continuar es T25 (ejecutor) y T26 (validador); aquí no se ejecuta nada");
    }
    Ok(Codigo::Exito.como_i32())
}

/// Lee el enunciado desde el estado durable, no de un archivo suelto.
///
/// Es deliberado: si la copia se hiciera desde un JSON que alguien pasa por la
/// línea de órdenes, nada garantizaría que es el mismo enunciado que el
/// coordinador registró, y el parche acabaría atribuido a una tarea que no es.
fn reanudacion(op: &Opciones) -> Result<state::Reanudacion, Error> {
    state::reanudar(&Host, op.exigido("estado")?, op.exigido("run")?)
}

fn manifiesto(captura: &str) -> Result<soso_improve_core::captura::Manifiesto, Error> {
    soso_improve_core::captura::leer_manifiesto(&Host, captura)
}

fn copiar(op: &Opciones) -> Result<i32, Error> {
    let r = reanudacion(op)?;
    let captura = op.exigido("captura")?;
    let destino = op.exigido("destino")?;
    let man = manifiesto(captura)?;

    std::fs::create_dir_all(destino)
        .map_err(|e| Error::entorno(format!("no pude crear {destino}: {e}")))?;
    let copia = workspace::preparar(
        &Host,
        captura,
        &mut Host,
        destino,
        &man,
        &r.spec,
        &r.manifiesto.run_id,
    )?;
    println!(
        "tarea: copia de {} en {} ({} archivo(s))",
        copia.task_id, copia.ruta, copia.archivos
    );
    println!("  base {}", copia.base);
    println!("  enunciado {} ({})", workspace::TASK_MD, copia.paquete_tarea);
    Ok(Codigo::Exito.como_i32())
}

fn exportar(op: &Opciones) -> Result<i32, Error> {
    let r = reanudacion(op)?;
    let man = manifiesto(op.exigido("captura")?)?;
    let copia = workspace::leer_marca(&Host, op.exigido("copia")?)?;
    if copia.run_id != r.manifiesto.run_id {
        return Err(Error::uso(format!(
            "esa copia es de {}, no de {}",
            copia.run_id, r.manifiesto.run_id
        )));
    }
    let politica = man.politica.clone();
    let exp = workspace::exportar(&Host, &copia, &man, &politica)?;

    for op_cambio in &exp.paquete.operaciones {
        println!("  {:12} {}", nombre_op(op_cambio), op_cambio.ruta());
    }
    // Lo rechazado se imprime siempre, aunque no haya nada: un informe que
    // calla cuando está vacío no distingue «no había enlaces» de «no miré».
    println!("  rechazados: {}", exp.rechazados.len());
    for x in &exp.rechazados {
        println!("    {} ({})", x.ruta, x.motivo);
    }
    println!(
        "tarea: {} cambio(s); {} ruta(s) sembradas por el coordinador no cuentan",
        exp.paquete.operaciones.len(),
        exp.previos.len()
    );

    if let Some(salida) = op.uno("out").filter(|s| !s.is_empty()) {
        let json = serde_json::to_vec_pretty(&exp.paquete)
            .map_err(|e| Error::formato(format!("no pude serializar el paquete: {e}")))?;
        std::fs::write(salida, &json)
            .map_err(|e| Error::entorno(format!("no pude escribir {salida}: {e}")))?;
        println!("  paquete en {salida}");
    }
    Ok(Codigo::Exito.como_i32())
}

fn nombre_op(o: &soso_improve_core::delta::Operacion) -> &'static str {
    use soso_improve_core::delta::Operacion::*;
    match o {
        Alta { .. } => "alta",
        Modificacion { .. } => "modificacion",
        Borrado { .. } => "borrado",
    }
}

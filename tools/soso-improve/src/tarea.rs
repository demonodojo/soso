//! `tarea` — las órdenes del coordinador (T23).
//!
//! Aquí sólo está la envoltura del host: leer y escribir archivos de verdad.
//! El formato, las transiciones y quién puede aceptar viven en
//! `soso_improve_core::state`, porque el guest tiene que aplicar exactamente el
//! mismo criterio. Si esa política se duplicara, una campaña diría cosas
//! distintas según dónde corriera.
//!
//! `ejecutar` (T25) corre el agente nativo sobre la copia; validar al candidato
//! `validar` (T26) reconstruye la base desde la captura, aplica el paquete y decide sólo por códigos de salida.

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
        "ejecutar" => crate::agente::ejecutar_tarea(op),
        "validar" => validar(op),
        "reanudar" if op.bandera("continuar") => continuar(op),
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
        println!("tarea: informar no continúa nada; `--continuar` reanuda (T27) con el diario, sin repetir efectos");
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

/// `tarea validar` (T26): exporta el candidato de la copia, lo envía al validador por contenido y
/// decide. Aquí el «validador» es otro directorio del mismo host: la separación por **instancia**
/// (receptor soso) es T43; el informe lo dice.
fn validar(op: &Opciones) -> Result<i32, Error> {
    use soso_improve_core::validate::{self, Exigencias, Sitios, Veredicto};
    let r = reanudacion(op)?;
    let captura = op.exigido("captura")?;
    let man = manifiesto(captura)?;
    let copia = workspace::leer_marca(&Host, op.exigido("copia")?)?;
    if copia.run_id != r.manifiesto.run_id {
        return Err(Error::uso(format!("esa copia es de {}, no de {}", copia.run_id, r.manifiesto.run_id)));
    }
    let ex: Exigencias = serde_json::from_slice(
        &std::fs::read(op.exigido("exigencias")?).map_err(|e| Error::entorno(format!("no pude leer las exigencias: {e}")))?,
    )
    .map_err(|e| Error::formato(format!("exigencias ilegibles: {e}")))?;
    let trabajo = op.exigido("trabajo")?.trim_end_matches('/').to_string();
    let referencias = op.exigido("referencias")?.to_string();
    let campana = op.exigido("campana")?;
    std::fs::create_dir_all(&trabajo).map_err(|e| Error::entorno(format!("no pude crear {trabajo}: {e}")))?;

    let mut host = Host;
    if op.bandera("publicar-base") {
        let g = validate::publicar_base(&mut host, &referencias, campana, &soso_improve_core::captura::huella(&man.inventario))?;
        println!("validar: base de «{campana}» publicada (generación {g})");
    }
    let politica = man.politica.clone();
    let exp = workspace::exportar(&Host, &copia, &man, &politica)?;
    if exp.vacia() {
        return Err(Error::uso("el candidato no cambió nada: no hay nada que validar"));
    }
    let objetos = format!("{trabajo}/objetos");
    let n = validate::empaquetar_objetos(&mut host, &copia.ruta, &exp.paquete, &objetos)?;
    println!("validar: paquete de {} operación(es), {n} objeto(s) enviados al validador", exp.paquete.operaciones.len());
    let arbol = format!("{trabajo}/arbol");
    let sitios = Sitios { captura, objetos: &objetos, arbol: &arbol, referencias: &referencias, campana };
    let inf = validate::validar_candidato(
        &mut host,
        &sitios,
        &exp.paquete,
        op.uno("paquete-hash").filter(|h| !h.is_empty()),
        &r.spec,
        &ex,
        Some((op.exigido("estado")?, op.exigido("run")?)),
    )?;
    print!("{}", validate::resumen(&inf));
    println!("validar: el validador comparte host con el agente; la separación por instancia es T43");
    Ok(match inf.veredicto {
        Veredicto::Aceptado => Codigo::Exito.como_i32(),
        Veredicto::Rechazado => Codigo::Verificacion.como_i32(),
        Veredicto::Bloqueado => Codigo::Error.como_i32(),
    })
}

/// `tarea reanudar --continuar` (T27): lleva la ejecución hasta el final desde donde se quedó, reconciliando el
/// diario con la evidencia (copia, hash del paquete, referencia de base). No repite lo ya hecho; si no puede
/// decidir qué hizo el agente al caerse, se bloquea y dice qué comprobar (`--adoptar-copia` lo resuelve).
fn continuar(op: &Opciones) -> Result<i32, Error> {
    use soso_improve_core::agente::{http::ModeloHttp, Config, Politica as PolAgente, SISTEMA};
    use soso_improve_core::recovery::{avanzar, Contexto, Desenlace};
    use soso_improve_core::transporte::Destino;
    use soso_improve_core::validate::{Exigencias, Sitios, Veredicto};
    let r = reanudacion(op)?;
    let captura = op.exigido("captura")?;
    let copia = std::fs::canonicalize(op.exigido("copia")?).map_err(|e| Error::uso(format!("--copia: {e}")))?.to_string_lossy().into_owned();
    let ex: Exigencias = serde_json::from_slice(&std::fs::read(op.exigido("exigencias")?).map_err(|e| Error::entorno(format!("exigencias: {e}")))?)
        .map_err(|e| Error::formato(format!("exigencias ilegibles: {e}")))?;
    let trabajo = op.exigido("trabajo")?.trim_end_matches('/').to_string();
    let referencias = op.exigido("referencias")?.to_string();
    let objetos = format!("{trabajo}/objetos");
    let arbol = format!("{trabajo}/arbol");
    std::fs::create_dir_all(&trabajo).map_err(|e| Error::entorno(format!("no pude crear {trabajo}: {e}")))?;
    let puerto: u16 = op.uno("puerto").and_then(|p| p.parse().ok()).unwrap_or(17299);
    let cfg = Config::nueva(op.uno("modelo").filter(|s| !s.is_empty()).unwrap_or("soso-coder"), SISTEMA);
    let mut pol = PolAgente::nueva(&copia);
    if let Some(h) = op.uno("cargo-home").filter(|s| !s.is_empty()) {
        pol.entorno.push(("CARGO_HOME".into(), h.to_string()));
    }
    let reloj = crate::sistema::RelojHost::default();
    let conector = crate::sistema::ConectorHost;
    let mut modelo = ModeloHttp {
        conector: &conector,
        reloj: &reloj,
        destino: Destino::local(puerto),
        plazo_conexion_ms: 10_000,
        token: op.uno("token").filter(|t| !t.is_empty()).map(|t| t.to_string()).or_else(|| std::env::var("SOSO_LLM_API_KEY").ok()),
    };
    let estado = op.exigido("estado")?.to_string();
    let run = op.exigido("run")?.to_string();
    let cx = Contexto {
        dir_estado: &estado,
        run_id: &run,
        captura,
        copia: &copia,
        sitios: Sitios { captura, objetos: &objetos, arbol: &arbol, referencias: &referencias, campana: op.exigido("campana")? },
        spec: &r.spec,
        exigencias: &ex,
        adoptar_copia: op.bandera("adoptar-copia"),
    };
    let mut host = Host;
    // La base de la campaña se publica la primera vez (sólo con la bandera): después la mueve únicamente el validador.
    if op.bandera("publicar-base") && soso_improve_core::validate::base_vigente(&host, &referencias, cx.sitios.campana)?.is_none() {
        let man = manifiesto(captura)?;
        let g = soso_improve_core::validate::publicar_base(&mut host, &referencias, cx.sitios.campana, &soso_improve_core::captura::huella(&man.inventario))?;
        println!("  base de «{}» publicada (generación {g})", cx.sitios.campana);
    }
    let res = avanzar(&mut host, &reloj, &cx, &mut |h: &mut Host| {
        let inf = soso_improve_core::runner::ejecutar_intento(h, &mut modelo, &reloj, &estado, &run, &copia, &cfg, &pol)?;
        if inf.fin != soso_improve_core::runner::Fin::Completo {
            return Err(Error::entorno(format!("el intento no terminó: {:?} {}", inf.fin, inf.detalle)));
        }
        Ok(())
    })?;
    for x in &res.reutilizadas {
        println!("  ya hecho: {x}");
    }
    for x in &res.ejecutadas {
        println!("  ejecutado: {x}");
    }
    match res.desenlace {
        Desenlace::Validado(Veredicto::Aceptado) => {
            println!("tarea: aceptada");
            Ok(Codigo::Exito.como_i32())
        }
        Desenlace::Validado(v) => {
            println!("tarea: {v:?}");
            Ok(Codigo::Verificacion.como_i32())
        }
        Desenlace::Bloqueado { motivo, comprobacion } => {
            println!("tarea: BLOQUEADA — {motivo}\n  comprobación necesaria: {comprobacion}");
            Ok(Codigo::Error.como_i32())
        }
    }
}

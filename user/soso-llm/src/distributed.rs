//! Sesión distribuida en estrella (protocolo v4): head muestrea; nodos ejecutan tramos.

use crate::net::{parse_sock_addr, TcpFd};
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{print, println, sys};
use soso_llm_core::generation::{GenerationOptions, GenerationObserver, GenCheckpoint};
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::pipeline::{
    self, AckPayload, BeginPayload, FramedTransport, HelloPayload, Message, NodeCaps, PipelinePlan,
    RecvError, DEFAULT_HANDSHAKE_TIMEOUT_MS, DEFAULT_PING_IDLE_MS, DEFAULT_PING_INTERVAL_MS,
    DEFAULT_STANDBY_RETRY_MS, DEFAULT_STEP_TIMEOUT_MS, ERR_NODE_DOWN, WIRE_F32,
};
use soso_llm_core::cluster::{
    Conector, EstrellaConfig, EstrellaHead, NodoConfig, NodoPipeline, PendingEnlace,
};
use soso_llm_core::remoto::ClusterGen;
use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::layer::TensorSource;
use soso_llm_core::pipeline::PipelineRole;
use soso_llm_core::tokenizer::{StreamDecoder, Tokenizer};
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

use crate::cluster;
use crate::session::{load_model_from_catalog, ModelBundle, Sesion};
use soso_llm_core::plan::bytes_por_capa;
use soso_llm_core::pipeline::WIRE_F16;

pub struct DistributedConfig {
    pub plan: PipelinePlan,
    pub remotes: Vec<String>,
    pub listen_port: u16,
    pub layer_start: u32,
    pub layer_end: u32,
    pub model_name: String,
    pub manifest_crc: u32,
    pub index_crc: u32,
    pub step_timeout_ms: u64,
    pub handshake_timeout_ms: u64,
    pub accept_timeout_ms: u64,
    pub ping_interval_ms: u64,
    pub ping_idle_ms: u64,
    pub standby: bool,
    pub standby_retry_ms: u64,
    pub wire: u8,
    pub node_auto: bool,
    /// Prueba: cerrar sesión tras N `Step` atendidos (`--salir-tras-steps`).
    pub salir_tras_steps: u32,
}

/// Head pre-enlazado (p. ej. tras `--splits auto`).
pub type PrelinkedHead = EstrellaHead<TcpConector>;

pub(crate) struct TcpConector;

impl Conector for TcpConector {
    type T = TcpFd;

    fn conectar(&mut self, addr: &str, timeout_ms: u64) -> Result<FramedTransport<TcpFd>, ()> {
        let sock = parse_sock_addr(addr).ok_or(())?;
        let stream = TcpFd::connect(&sock, timeout_ms).map_err(|e| {
            println!("soso-llm: tcp_connect a {addr} falló ({e})");
            ()
        })?;
        Ok(FramedTransport::new(stream))
    }
}

fn estrella_cfg(cfg: &DistributedConfig) -> EstrellaConfig {
    EstrellaConfig {
        wire: cfg.wire,
        step_timeout_ms: cfg.step_timeout_ms,
        handshake_timeout_ms: cfg.handshake_timeout_ms,
        ping_interval_ms: cfg.ping_interval_ms,
        ping_idle_ms: cfg.ping_idle_ms,
        standby_retry_ms: cfg.standby_retry_ms,
        retry_max: 3,
        strict_node_layers: !cfg.node_auto,
        model_name: cfg.model_name.clone(),
        manifest_crc: cfg.manifest_crc,
        index_crc: cfg.index_crc,
    }
}

fn now_ms() -> u64 {
    sys::uptime_ms().max(0) as u64
}

pub(crate) fn session_id_for_seed(seed: u64) -> u64 {
    now_ms()
        .wrapping_mul(0x517c_c1b7)
        .wrapping_add(seed)
}

fn new_session_id(seed: u64) -> u64 {
    session_id_for_seed(seed)
}

/// Tras failover, el head nuevo anuncia `kv_pos` / `prefix_hash` en el `Hello`.
fn aplicar_hello_kv_failover(
    hello: &HelloPayload,
    nodo: &mut NodoPipeline,
    rt: &mut Runtime,
) -> Result<(), ()> {
    if hello.kv_pos == 0 {
        return Ok(());
    }
    if hello.prefix_hash != 0
        && nodo.prefix_hash != 0
        && hello.prefix_hash != nodo.prefix_hash
    {
        return Err(());
    }
    let pos = hello.kv_pos as usize;
    if rt.pos != pos {
        rt.truncar_kv(pos);
    }
    nodo.kv_pos = hello.kv_pos;
    nodo.prefix_hash = hello.prefix_hash;
    Ok(())
}

fn log_recv_err(context: &str, e: RecvError) {
    match e {
        RecvError::Timeout => println!("soso-llm: timeout en {context}"),
        RecvError::Disconnected => println!("soso-llm: desconexión en {context}"),
        RecvError::Protocol => println!("soso-llm: protocolo inválido en {context}"),
    }
}

fn recv_msg(
    tx: &mut FramedTransport<TcpFd>,
    timeout_ms: u64,
    keepalive: bool,
    cfg: &DistributedConfig,
    context: &str,
) -> Result<Message, ()> {
    let result = if keepalive && cfg.ping_interval_ms != 0 {
        tx.recv_timeout_keepalive(
            timeout_ms,
            cfg.ping_interval_ms,
            cfg.ping_idle_ms,
            now_ms,
        )
    } else {
        tx.recv_timeout(timeout_ms, now_ms)
    };
    result.map_err(|e| {
        log_recv_err(context, e);
        ()
    })
}

struct PrintObs<'a> {
    dec: StreamDecoder,
    tok: &'a Tokenizer,
}

impl GenerationObserver for PrintObs<'_> {
    fn on_token(&mut self, token: u32) {
        let s = self.dec.push(self.tok, token);
        if !s.is_empty() {
            print!("{s}");
        }
    }

    fn cancel_at(&mut self, _: GenCheckpoint) -> bool {
        false
    }
}

/// Head en modo standby: reintenta hasta tomar el control del cluster tras failover.
pub fn run_head_standby(
    rt: &mut Runtime,
    source: &mut impl TensorSource,
    tok: &Tokenizer,
    cfg: &DistributedConfig,
    prompt: &str,
    max_new: usize,
    sampler: &mut Sampler,
    seed: u64,
    par: Option<&dyn RowParallel>,
) -> Result<(), ()> {
    println!(
        "soso-llm: head standby activo (reintento cada {} ms)",
        cfg.standby_retry_ms
    );
    loop {
        println!("soso-llm: standby buscando nodos libres...");
        match run_head(
            rt,
            source,
            tok,
            cfg,
            prompt,
            max_new,
            sampler,
            seed,
            par,
            None,
        ) {
            Ok(()) => println!("soso-llm: inferencia completada"),
            Err(()) => println!("soso-llm: nodos ocupados o sesión fallida"),
        }
        rt.reset_sequence();
        sys::sleep_ms(cfg.standby_retry_ms);
    }
}

pub fn run_head(
    rt: &mut Runtime,
    source: &mut impl TensorSource,
    tok: &Tokenizer,
    cfg: &DistributedConfig,
    prompt: &str,
    max_new: usize,
    sampler: &mut Sampler,
    seed: u64,
    par: Option<&dyn RowParallel>,
    pre_estrella: Option<PrelinkedHead>,
) -> Result<(), ()> {
    if cfg.remotes.len() != cfg.plan.remote_count() {
        return Err(());
    }

    let head_seg = cfg.plan.head_segment();
    let session_id = new_session_id(seed);

    let mut estrella = match pre_estrella {
        Some(e) => e,
        None => {
            let mut e = EstrellaHead::new(
                estrella_cfg(cfg),
                cfg.plan.clone(),
                TcpConector,
                rt.manifest.hidden_dim,
                rt.manifest.num_layers,
                now_ms,
            );
            e.enlazar(
                &cfg.remotes,
                rt.pos as u32,
                soso_llm_core::prefix_hash_tokens(&[]),
            )?;
            e
        }
    };
    let begin = BeginPayload {
        temp: sampler.temp,
        top_p: sampler.top_p,
        seed,
        max_new: max_new as u32,
        session_id,
    };
    estrella.begin_session(begin)?;

    let mut cluster_gen = ClusterGen {
        local_end: head_seg.layer_end,
        remoto: &mut estrella,
        journal: None,
        prefix_hash: 0,
    };
    let mut cluster_slot = Some(&mut cluster_gen);

    rt.reset_sequence();
    let text = if prompt.is_empty() { "test" } else { prompt };
    let prompt_ids = tok.encode(text);
    if prompt_ids.is_empty() {
        abort_estrella(&mut estrella, ERR_NODE_DOWN, "prompt vacío");
        return Err(());
    }

    let mut obs = PrintObs {
        dec: StreamDecoder::new(),
        tok,
    };
    rt.reloj = Some(now_ms);
    let stops = soso_llm_core::stops_from_eos(tok.eos());
    let options = GenerationOptions::new(max_new, stops);
    let result = rt.generate_stream_planned_observed(
        source,
        &prompt_ids,
        options,
        sampler,
        &mut obs,
        par,
        &mut None,
        now_ms,
        &mut || soso_llm_core::plan::MemSnapshot::default(),
        &mut cluster_slot,
    );

    match result {
        Ok((tokens, _)) => {
            let resto = obs.dec.finish();
            if !resto.is_empty() {
                print!("{resto}");
            }
            println!();
            println!(
                "soso-llm: generado distribuido ({} tokens) | último salto {} ms, {} bytes cable",
                tokens.len(),
                estrella.ultimo_ms(),
                estrella.ultimo_bytes()
            );
            Ok(())
        }
        Err(()) => {
            abort_estrella(&mut estrella, ERR_NODE_DOWN, "generación fallida");
            Err(())
        }
    }
}

fn abort_estrella(estrella: &mut EstrellaHead<TcpConector>, code: u16, msg: &str) {
    for link in estrella.links.iter_mut() {
        let _ = link.tx.send(&Message::Error(pipeline::ErrorPayload {
            code,
            message: String::from(msg),
        }));
    }
}

/// Nodo `--auto`: catálogo al arrancar; carga pesos al recibir `Assign`.
pub fn run_node_auto(
    name: &str,
    manifest: Manifest,
    index: TensorIndex,
    manifest_crc: u32,
    index_crc: u32,
    cfg: &DistributedConfig,
    par: Option<&dyn RowParallel>,
) -> u8 {
    let listener = match TcpFd::listen(cfg.listen_port) {
        Ok(l) => l,
        Err(_) => return 1,
    };
    let accept_ms = cfg.accept_timeout_ms;
    println!(
        "soso-llm: nodo --auto en :{} (esperando Assign)",
        cfg.listen_port
    );
    let mut bundle: Option<ModelBundle> = None;
    loop {
        let stream = match TcpFd::accept(&listener, accept_ms) {
            Ok(s) => s,
            Err(e) if e == -(soso_abi::EAGAIN as i64) => continue,
            Err(e) => {
                println!("soso-llm: accept falló ({e})");
                return 1;
            }
        };
        let r = run_node_session_auto(
            name,
            &manifest,
            &index,
            manifest_crc,
            index_crc,
            &mut bundle,
            cfg,
            par,
            stream,
        );
        match r {
            Ok(()) => println!("soso-llm: sesión finalizada, re-escuchando"),
            Err(()) => println!("soso-llm: sesión abortada, re-escuchando"),
        }
    }
}

fn run_node_session_auto(
    name: &str,
    manifest: &Manifest,
    index: &TensorIndex,
    manifest_crc: u32,
    index_crc: u32,
    bundle: &mut Option<ModelBundle>,
    cfg: &DistributedConfig,
    par: Option<&dyn RowParallel>,
    stream: TcpFd,
) -> Result<(), ()> {
    let stub = Runtime::new(manifest.clone(), index.clone(), 32 * 1024 * 1024, 0);
    let _ = (manifest_crc, index_crc);
    let hs_to = cfg.handshake_timeout_ms;
    let step_to = cfg.step_timeout_ms;
    let mut rx = FramedTransport::new(stream);
    let mut nodo = NodoPipeline::new(NodoConfig {
        layer_start: cfg.layer_start,
        layer_end: cfg.layer_end,
        fixed_layers: false,
        model_name: cfg.model_name.clone(),
        manifest_crc: cfg.manifest_crc,
        index_crc: cfg.index_crc,
    });
    let hello = match recv_msg(&mut rx, hs_to, true, cfg, "handshake head")? {
        Message::Hello(h) => h,
        _ => return Err(()),
    };
    if !NodoPipeline::hello_head_valido(
        &hello,
        &nodo.cfg,
        stub.manifest.num_layers,
        stub.manifest.hidden_dim,
    ) {
        let _ = rx.send(&NodoPipeline::error_handshake("handshake incompatible"));
        return Err(());
    }
    rx.send(&Message::Hello(nodo.hello_nodo(
        &stub,
        stub.manifest.num_layers,
        stub.manifest.hidden_dim,
        stub.manifest.vocab_size,
    )))?;
    rx.send(&Message::Caps(node_caps()))?;
    let assign = match recv_msg(&mut rx, hs_to, true, cfg, "assign")? {
        Message::Assign(a) => a,
        Message::Error(_) => return Err(()),
        _ => return Err(()),
    };
    match nodo.apply_assign(assign) {
        Ok(()) => {}
        Err(m) => {
            rx.send(&m)?;
            return Err(());
        }
    }
    let ls = nodo.cfg.layer_start;
    let le = nodo.cfg.layer_end;
    let role = PipelineRole::from_layer_range(ls, le, stub.manifest.num_layers);
    let reload = bundle
        .as_ref()
        .map(|b| b.rt.manifest.num_layers != stub.manifest.num_layers)
        .unwrap_or(true)
        || bundle.as_ref().is_none();
    if reload {
        if let Some(mut b) = bundle.take() {
            b.source.shutdown_worker();
        }
        let loaded = load_model_from_catalog(
            name,
            manifest.clone(),
            index.clone(),
            manifest_crc,
            index_crc,
            role,
            ls,
            le,
            true,
        )
        .map_err(|_| ())?;
        *bundle = Some(loaded);
        println!("soso-llm: modelo cargado capas {ls}..{le}");
    }
    let b = bundle.as_mut().ok_or(())?;
    aplicar_hello_kv_failover(&hello, &mut nodo, &mut b.rt)?;
    rx.send(&Message::Ack(AckPayload { pos: 0 }))?;
    let begin = match recv_msg(&mut rx, hs_to, true, cfg, "begin")? {
        Message::Begin(b) => b,
        Message::Error(_) => return Err(()),
        _ => return Err(()),
    };
    rx.send(&Message::Ack(AckPayload { pos: 0 }))?;
    let reset_kv = hello.kv_pos == 0 && b.rt.pos == 0;
    nodo.procesar_begin(begin, &mut b.rt, reset_kv);
    let mut steps_atendidos = 0u32;
    loop {
        let msg = recv_msg(&mut rx, step_to, true, cfg, "step")?;
        match msg {
            Message::Error(e) => {
                println!("soso-llm: head abortó sesión: {}", e.message);
                return Err(());
            }
            other => {
                let es_step = matches!(other, Message::Step(_));
                match nodo.atender(&mut b.rt, &mut b.source, par, other)? {
                    Some(reply) => rx.send(&reply)?,
                    None => return Err(()),
                }
                if es_step {
                    steps_atendidos += 1;
                    if cfg.salir_tras_steps != 0 && steps_atendidos >= cfg.salir_tras_steps {
                        println!(
                            "soso-llm: --salir-tras-steps {} alcanzado",
                            cfg.salir_tras_steps
                        );
                        return Err(());
                    }
                }
            }
        }
    }
}

/// Nodo persistente: re-escucha tras caída del head o timeout de sesión.
pub fn run_node(
    rt: &mut Runtime,
    source: &mut impl TensorSource,
    cfg: &DistributedConfig,
    par: Option<&dyn RowParallel>,
) -> Result<(), ()> {
    let listener = TcpFd::listen(cfg.listen_port).map_err(|_| ())?;
    let accept_ms = cfg.accept_timeout_ms;
    println!(
        "soso-llm: nodo escuchando en :{} (capas {}..{}), protocolo v4",
        cfg.listen_port, cfg.layer_start, cfg.layer_end
    );

    loop {
        let stream = match TcpFd::accept(&listener, accept_ms) {
            Ok(s) => s,
            Err(e) if e == -(soso_abi::EAGAIN as i64) => continue,
            Err(e) => {
                println!("soso-llm: accept falló ({e})");
                return Err(());
            }
        };
        match run_node_session(rt, source, cfg, par, stream) {
            Ok(()) => println!("soso-llm: sesión finalizada, re-escuchando"),
            Err(()) => println!("soso-llm: sesión abortada, re-escuchando"),
        }
    }
}

fn run_node_session(
    rt: &mut Runtime,
    source: &mut impl TensorSource,
    cfg: &DistributedConfig,
    par: Option<&dyn RowParallel>,
    stream: TcpFd,
) -> Result<(), ()> {
    let hs_to = cfg.handshake_timeout_ms;
    let step_to = cfg.step_timeout_ms;
    let layer_start = cfg.layer_start;
    let layer_end = cfg.layer_end;
    let mut rx = FramedTransport::new(stream);
    let mut nodo = NodoPipeline::new(NodoConfig {
        layer_start,
        layer_end,
        fixed_layers: !cfg.node_auto,
        model_name: cfg.model_name.clone(),
        manifest_crc: cfg.manifest_crc,
        index_crc: cfg.index_crc,
    });
    let hello = match recv_msg(&mut rx, hs_to, true, cfg, "handshake head")? {
        Message::Hello(h) => h,
        _ => return Err(()),
    };
    if !NodoPipeline::hello_head_valido(
        &hello,
        &nodo.cfg,
        rt.manifest.num_layers,
        rt.manifest.hidden_dim,
    ) {
        let _ = rx.send(&NodoPipeline::error_handshake("handshake incompatible"));
        return Err(());
    }
    aplicar_hello_kv_failover(&hello, &mut nodo, rt)?;

    let caps = node_caps();
    rx.send(&Message::Hello(nodo.hello_nodo(
        rt,
        rt.manifest.num_layers,
        rt.manifest.hidden_dim,
        rt.manifest.vocab_size,
    )))?;
    rx.send(&Message::Caps(caps))?;

    let assign = match recv_msg(&mut rx, hs_to, true, cfg, "assign")? {
        Message::Assign(a) => a,
        Message::Error(_) => return Err(()),
        _ => return Err(()),
    };
    match nodo.apply_assign(assign) {
        Ok(()) => {}
        Err(m) => {
            rx.send(&m)?;
            return Err(());
        }
    }
    rx.send(&Message::Ack(AckPayload { pos: 0 }))?;

    let begin = match recv_msg(&mut rx, hs_to, true, cfg, "begin")? {
        Message::Begin(b) => b,
        Message::Error(_) => return Err(()),
        _ => return Err(()),
    };
    println!(
        "soso-llm: sesión {} iniciada (max_new={})",
        begin.session_id, begin.max_new
    );
    rx.send(&Message::Ack(AckPayload { pos: 0 }))?;
    let reset_kv = hello.kv_pos == 0 && rt.pos == 0;
    nodo.procesar_begin(begin, rt, reset_kv);

    let mut steps_atendidos = 0u32;
    loop {
        let msg = recv_msg(&mut rx, step_to, true, cfg, "step")?;
        match msg {
            Message::Error(e) => {
                println!("soso-llm: head abortó sesión: {}", e.message);
                return Err(());
            }
            other => {
                let es_step = matches!(other, Message::Step(_));
                match nodo.atender(rt, source, par, other)? {
                    Some(reply) => rx.send(&reply)?,
                    None => return Err(()),
                }
                if es_step {
                    steps_atendidos += 1;
                    if cfg.salir_tras_steps != 0 && steps_atendidos >= cfg.salir_tras_steps {
                        println!(
                            "soso-llm: --salir-tras-steps {} alcanzado",
                            cfg.salir_tras_steps
                        );
                        return Err(());
                    }
                }
            }
        }
    }
}

fn node_caps() -> NodeCaps {
    let mut info = soso_abi::MemInfo::default();
    let ram_mb = if sys::meminfo(&mut info) == 0 {
        (info.free_frames.saturating_mul(4096) / (1024 * 1024)) as u32
    } else {
        512
    };
    NodeCaps {
        ram_free_mb: ram_mb,
        workers: 1,
        bw_score: 1000,
    }
}

pub fn crc_bytes(data: &[u8]) -> u32 {
    pipeline::crc32c(data)
}

pub fn default_timeouts() -> (u64, u64, u64) {
    (
        DEFAULT_STEP_TIMEOUT_MS,
        DEFAULT_HANDSHAKE_TIMEOUT_MS,
        DEFAULT_HANDSHAKE_TIMEOUT_MS,
    )
}

pub fn default_keepalive() -> (u64, u64, u64) {
    (
        DEFAULT_PING_INTERVAL_MS,
        DEFAULT_PING_IDLE_MS,
        DEFAULT_STANDBY_RETRY_MS,
    )
}

pub(crate) fn parse_split_list(splits: &str) -> Vec<u32> {
    splits
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect()
}

pub(crate) fn parse_pipeline_list(pipeline: &str) -> Vec<String> {
    pipeline
        .split(',')
        .map(|s| String::from(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

pub(crate) fn wire_from_str(s: &str) -> u8 {
    if s == "f16" {
        WIRE_F16
    } else {
        WIRE_F32
    }
}

pub(crate) fn head_caps() -> NodeCaps {
    let mut info = soso_abi::MemInfo::default();
    let ram_mb = if sys::meminfo(&mut info) == 0 {
        (info.free_frames.saturating_mul(4096) / (1024 * 1024)) as u32
    } else {
        512
    };
    NodeCaps {
        ram_free_mb: ram_mb,
        workers: 1,
        bw_score: 1000,
    }
}

/// Enlaza el head con los nodos (`serve --cluster`, askd `cluster=`). Sin `Begin` (va por petición).
pub(crate) fn begin_turno_cluster(
    sesion: &mut Sesion,
    max_new: u32,
    temp: f32,
    top_p: f32,
    seed: u64,
) -> Result<(), u8> {
    let Some(est) = sesion.cluster.as_mut() else {
        return Ok(());
    };
    est.begin_session(BeginPayload {
        temp,
        top_p,
        seed,
        max_new,
        session_id: session_id_for_seed(seed),
    })
    .map_err(|_| 1u8)?;
    sesion.bundle.rt.reset_sequence();
    Ok(())
}

pub(crate) fn montar_head_enlaces(
    model_name: &str,
    manifest_crc: u32,
    index_crc: u32,
    hidden_dim: u32,
    num_layers: u32,
    index: &TensorIndex,
    pipeline: &str,
    splits: &str,
    wire: u8,
    cluster_port: u16,
) -> Result<(PrelinkedHead, PipelinePlan), u8> {
    let remotes = if pipeline == "auto" {
        println!("soso-llm: descubriendo nodos en /24…");
        cluster::descubrir_nodos(cluster_port)
    } else {
        parse_pipeline_list(pipeline)
    };
    if remotes.is_empty() {
        println!("soso-llm: no hay nodos remotos");
        return Err(1);
    }
    let bytes_capa = bytes_por_capa(index, num_layers);
    let (step_to, hs_to, accept_to) = default_timeouts();
    let (ping_int, ping_idle, _) = default_keepalive();
    let probe_cfg = DistributedConfig {
        plan: PipelinePlan {
            segments: alloc::vec![soso_llm_core::pipeline::PipelineSegment {
                layer_start: 0,
                layer_end: num_layers,
            }],
        },
        remotes: remotes.clone(),
        listen_port: cluster_port,
        layer_start: 0,
        layer_end: num_layers,
        model_name: String::from(model_name),
        manifest_crc,
        index_crc,
        step_timeout_ms: step_to,
        handshake_timeout_ms: hs_to,
        accept_timeout_ms: accept_to,
        ping_interval_ms: ping_int,
        ping_idle_ms: ping_idle,
        standby: false,
        standby_retry_ms: DEFAULT_STANDBY_RETRY_MS,
        wire,
        node_auto: true,
        salir_tras_steps: 0,
    };
    let splits_eff = if splits.is_empty() { "auto" } else { splits };
    let (plan, estrella) = if splits_eff == "auto" {
        let (mut est, pending) = sondeo_caps_enlaces(&probe_cfg, &remotes, hidden_dim, num_layers)
            .map_err(|_| {
                println!("soso-llm: handshake de capacidades falló");
                1u8
            })?;
        let mut caps = alloc::vec![head_caps()];
        caps.extend(pending.iter().map(|p| p.caps));
        let plan = PipelinePlan::desde_capacidades(&caps, num_layers, bytes_capa).map_err(|_| {
            println!("soso-llm: reparto auto imposible");
            1u8
        })?;
        completar_enlaces_auto(&mut est, plan.clone(), pending, 0, 0).map_err(|_| {
            println!("soso-llm: assign tras reparto auto falló");
            1u8
        })?;
        (plan, est)
    } else {
        let split_vals = parse_split_list(splits_eff);
        let plan = PipelinePlan::from_splits(&split_vals, num_layers).map_err(|_| {
            println!("soso-llm: splits inválidos");
            1u8
        })?;
        let head = plan.head_segment();
        let dcfg = DistributedConfig {
            plan: plan.clone(),
            remotes: remotes.clone(),
            listen_port: 0,
            layer_start: head.layer_start,
            layer_end: head.layer_end,
            model_name: String::from(model_name),
            manifest_crc,
            index_crc,
            step_timeout_ms: step_to,
            handshake_timeout_ms: hs_to,
            accept_timeout_ms: accept_to,
            ping_interval_ms: ping_int,
            ping_idle_ms: ping_idle,
            standby: false,
            standby_retry_ms: DEFAULT_STANDBY_RETRY_MS,
            wire,
            node_auto: false,
            salir_tras_steps: 0,
        };
        let mut est = EstrellaHead::new(
            estrella_cfg(&dcfg),
            plan.clone(),
            TcpConector,
            hidden_dim,
            num_layers,
            now_ms,
        );
        est.enlazar(&remotes, 0, soso_llm_core::prefix_hash_tokens(&[]))
            .map_err(|_| {
                println!("soso-llm: enlazar nodos falló");
                1u8
            })?;
        (plan, est)
    };
    if remotes.len() != plan.remote_count() {
        println!(
            "soso-llm: pipeline tiene {} nodos pero splits implican {}",
            remotes.len(),
            plan.remote_count()
        );
        return Err(1);
    }
    Ok((estrella, plan))
}

/// Hello+Caps sobre conexiones que luego reciben `Assign` (`--splits auto`).
pub fn sondeo_caps_enlaces(
    cfg: &DistributedConfig,
    remotes: &[String],
    hidden_dim: u32,
    num_layers: u32,
) -> Result<(PrelinkedHead, Vec<PendingEnlace<TcpFd>>), ()> {
    let head_only = PipelinePlan {
        segments: alloc::vec![soso_llm_core::pipeline::PipelineSegment {
            layer_start: 0,
            layer_end: num_layers,
        }],
    };
    let mut est = EstrellaHead::new(
        estrella_cfg(cfg),
        head_only,
        TcpConector,
        hidden_dim,
        num_layers,
        now_ms,
    );
    let pending = est.recolectar_caps(remotes)?;
    Ok((est, pending))
}

pub fn completar_enlaces_auto(
    est: &mut PrelinkedHead,
    plan: PipelinePlan,
    pending: Vec<PendingEnlace<TcpFd>>,
    kv_pos: u32,
    prefix_hash: u32,
) -> Result<(), ()> {
    est.set_plan(plan);
    est.assignar_pending(pending, kv_pos, prefix_hash)
}

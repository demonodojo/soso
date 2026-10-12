//! Sesión distribuida en estrella: head orquesta N nodos remotos (v3 con tolerancia a fallos).

use crate::net::{parse_sock_addr, TcpFd};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{print, println, sys};
use soso_llm_core::parallel::RowParallel;
use soso_llm_core::pipeline::{
    self, AckPayload, BeginPayload, FramedTransport, HelloPayload, Message, PipelinePlan,
    RecvError, StepPayload, StepReplyPayload, DEFAULT_HANDSHAKE_TIMEOUT_MS,
    DEFAULT_PING_IDLE_MS, DEFAULT_PING_INTERVAL_MS, DEFAULT_STANDBY_RETRY_MS,
    DEFAULT_STEP_TIMEOUT_MS, ERR_NODE_DOWN, ROLE_HEAD, ROLE_NODE,
};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::layer::TensorSource;
use soso_llm_core::tokenizer::{StreamDecoder, Tokenizer};

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
}

struct RemoteLink {
    addr: String,
    tx: FramedTransport<TcpFd>,
    _segment: soso_llm_core::pipeline::PipelineSegment,
}

fn now_ms() -> u64 {
    sys::uptime_ms().max(0) as u64
}

fn new_session_id(seed: u64) -> u64 {
    now_ms()
        .wrapping_mul(0x517c_c1b7)
        .wrapping_add(seed)
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

/// Head en modo standby: reintenta hasta tomar el control del cluster tras failover.
pub fn run_head_standby(
    rt: &mut Runtime,
    source: &mut impl TensorSource,
    tok: &Tokenizer,
    cfg: &DistributedConfig,
    prompt: &str,
    max_new: usize,
    mut sampler: Sampler,
    seed: u64,
    par: Option<&dyn RowParallel>,
) -> Result<(), ()> {
    println!(
        "soso-llm: head standby activo (reintento cada {} ms)",
        cfg.standby_retry_ms
    );
    loop {
        println!("soso-llm: standby buscando nodos libres...");
        match run_head(rt, source, tok, cfg, prompt, max_new, &mut sampler, seed, par) {
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
) -> Result<(), ()> {
    if cfg.remotes.len() != cfg.plan.remote_count() {
        return Err(());
    }

    let head_seg = cfg.plan.head_segment();
    let hs_to = cfg.handshake_timeout_ms;
    let step_to = cfg.step_timeout_ms;
    let session_id = new_session_id(seed);

    let mut remotes = Vec::with_capacity(cfg.remotes.len());
    for (i, remote) in cfg.remotes.iter().enumerate() {
        let seg = cfg.plan.remote_segment(i).ok_or(())?;
        let addr = parse_sock_addr(remote).ok_or(())?;
        let stream = TcpFd::connect(&addr, hs_to).map_err(|e| {
            println!("soso-llm: tcp_connect a {remote} falló ({e})");
            ()
        })?;
        let mut tx = FramedTransport::new(stream);
        let hello = HelloPayload {
            role: ROLE_HEAD,
            layer_start: seg.layer_start,
            layer_end: seg.layer_end,
            num_layers: rt.manifest.num_layers,
            hidden_dim: rt.manifest.hidden_dim,
            vocab_size: rt.manifest.vocab_size,
            model_name: cfg.model_name.clone(),
            manifest_crc: cfg.manifest_crc,
            index_crc: cfg.index_crc,
        };
        tx.send(&Message::Hello(hello.clone()))?;
        match recv_msg(&mut tx, hs_to, true, cfg, "handshake remoto")? {
            Message::Hello(h) => {
                if !hello_matches_node(&h, cfg, seg, rt.manifest.num_layers) {
                    println!("soso-llm: handshake incompatible con {remote}");
                    return Err(());
                }
            }
            Message::Error(e) => {
                println!("soso-llm: nodo rechazó handshake: {}", e.message);
                return Err(());
            }
            _ => return Err(()),
        }
        remotes.push(RemoteLink {
            addr: remote.clone(),
            tx,
            _segment: seg,
        });
    }

    let begin = BeginPayload {
        temp: 0.0,
        top_p: sampler.top_p,
        seed,
        max_new: max_new as u32,
        session_id,
    };
    for link in remotes.iter_mut() {
        link.tx.send(&Message::Begin(begin))?;
    }
    for link in remotes.iter_mut() {
        match recv_msg(&mut link.tx, hs_to, true, cfg, "ack begin")? {
            Message::Ack { .. } | Message::Reset => {}
            Message::Error(e) => {
                println!("soso-llm: error remoto: {}", e.message);
                return Err(());
            }
            _ => return Err(()),
        }
    }

    rt.reset_sequence();
    let prompt_tokens = tok.encode(prompt);
    if prompt_tokens.is_empty() {
        return Err(());
    }

    let mut decoder = StreamDecoder::new();
    let mut tokens = prompt_tokens.clone();

    for (i, &t) in prompt_tokens.iter().enumerate() {
        let is_last_prompt = i + 1 == prompt_tokens.len();
        let want_token = is_last_prompt && max_new > 0;
        if run_head_step(rt, source, head_seg, t, want_token, &mut remotes, step_to, cfg, par)
            .is_err()
        {
            abort_remotes(&mut remotes, ERR_NODE_DOWN, "nodo no respondió");
            return Err(());
        }
        if want_token {
            let next =
                recv_token_from_chain(&mut remotes, rt.pos as u32 - 1, step_to, cfg)?.ok_or(())?;
            if tok.eos() == Some(next) {
                break;
            }
            tokens.push(next);
            let s = decoder.push(tok, next);
            if !s.is_empty() {
                print!("{s}");
            }
        }
    }

    for _ in 1..max_new {
        if rt.pos >= rt.manifest.max_seq as usize {
            break;
        }
        let last = *tokens.last().ok_or(())?;
        if run_head_step(rt, source, head_seg, last, true, &mut remotes, step_to, cfg, par)
            .is_err()
        {
            abort_remotes(&mut remotes, ERR_NODE_DOWN, "nodo no respondió");
            return Err(());
        }
        let next = recv_token_from_chain(&mut remotes, rt.pos as u32 - 1, step_to, cfg)?.ok_or(())?;
        if tok.eos() == Some(next) {
            break;
        }
        tokens.push(next);
        let s = decoder.push(tok, next);
        if !s.is_empty() {
            print!("{s}");
        }
    }

    let resto = decoder.finish();
    if !resto.is_empty() {
        print!("{resto}");
    }
    println!();
    println!("soso-llm: generado distribuido ({} tokens)", tokens.len());
    Ok(())
}

fn abort_remotes(remotes: &mut [RemoteLink], code: u16, msg: &str) {
    for link in remotes.iter_mut() {
        let _ = link.tx.send(&Message::Error(pipeline::ErrorPayload {
            code,
            message: String::from(msg),
        }));
    }
}

fn hello_matches_node(
    h: &HelloPayload,
    cfg: &DistributedConfig,
    seg: soso_llm_core::pipeline::PipelineSegment,
    num_layers: u32,
) -> bool {
    h.role == ROLE_NODE
        && h.layer_start == seg.layer_start
        && h.layer_end == seg.layer_end
        && h.num_layers == num_layers
        && h.model_name == cfg.model_name
        && h.manifest_crc == cfg.manifest_crc
        && h.index_crc == cfg.index_crc
}

fn run_head_step(
    rt: &mut Runtime,
    source: &mut impl TensorSource,
    head_seg: soso_llm_core::pipeline::PipelineSegment,
    token: u32,
    want_token: bool,
    remotes: &mut [RemoteLink],
    step_timeout_ms: u64,
    cfg: &DistributedConfig,
    par: Option<&dyn RowParallel>,
) -> Result<(), ()> {
    rt.embed_token(token, source)?;
    if head_seg.layer_end > head_seg.layer_start {
        rt.forward_layers_range(head_seg.layer_start, head_seg.layer_end, source, par, &mut None)?;
    }
    let mut hidden = rt.hidden_slice().to_vec();
    let pos = rt.pos as u32;
    let remote_count = remotes.len();

    for (i, link) in remotes.iter_mut().enumerate() {
        let is_tail = i + 1 == remote_count;
        link.tx.send(&Message::Step(StepPayload {
            pos,
            want_token: if is_tail && want_token { 1 } else { 0 },
            hidden,
        }))?;
        if is_tail && want_token {
            break;
        }
        // El nodo está calculando: solo timeout total, sin keepalive idle.
        let t0 = now_ms();
        match recv_msg(
            &mut link.tx,
            step_timeout_ms,
            false,
            cfg,
            &format!("step pos {pos} en {}", link.addr),
        )? {
            Message::StepReply(r) => {
                if r.pos != pos {
                    return Err(());
                }
                hidden = r.hidden;
                if let Some(pl) = rt.planner.as_mut() {
                    pl.observe_remote_rtt(now_ms().saturating_sub(t0));
                }
            }
            Message::Error(e) => {
                println!("soso-llm: error remoto ({}): {}", link.addr, e.message);
                return Err(());
            }
            _ => return Err(()),
        }
    }

    rt.advance_pos();
    Ok(())
}

fn recv_token_from_chain(
    remotes: &mut [RemoteLink],
    pos: u32,
    step_timeout_ms: u64,
    cfg: &DistributedConfig,
) -> Result<Option<u32>, ()> {
    let tail = remotes.last_mut().ok_or(())?;
    match recv_msg(
        &mut tail.tx,
        step_timeout_ms,
        false,
        cfg,
        &format!("token pos {pos} en {}", tail.addr),
    )? {
        Message::Token(t) => {
            if t.pos != pos {
                return Err(());
            }
            Ok(Some(t.token))
        }
        Message::Error(e) => {
            println!("soso-llm: error remoto: {}", e.message);
            Err(())
        }
        _ => Err(()),
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
        "soso-llm: nodo en :{} (capas {}..{}), tolerante a fallos",
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
        rt.reset_sequence();
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
    let is_tail = cfg.layer_end == rt.manifest.num_layers;
    let mut rx = FramedTransport::new(stream);

    let hello = match recv_msg(&mut rx, hs_to, true, cfg, "handshake head")? {
        Message::Hello(h) => h,
        _ => return Err(()),
    };
    if hello.role != ROLE_HEAD
        || hello.layer_start != cfg.layer_start
        || hello.layer_end != cfg.layer_end
        || hello.num_layers != rt.manifest.num_layers
        || hello.hidden_dim != rt.manifest.hidden_dim
        || hello.model_name != cfg.model_name
        || hello.manifest_crc != cfg.manifest_crc
        || hello.index_crc != cfg.index_crc
    {
        let _ = rx.send(&Message::Error(pipeline::ErrorPayload {
            code: pipeline::ERR_HANDSHAKE,
            message: String::from("handshake incompatible"),
        }));
        return Err(());
    }

    rx.send(&Message::Hello(HelloPayload {
        role: ROLE_NODE,
        layer_start: cfg.layer_start,
        layer_end: cfg.layer_end,
        num_layers: rt.manifest.num_layers,
        hidden_dim: rt.manifest.hidden_dim,
        vocab_size: rt.manifest.vocab_size,
        model_name: cfg.model_name.clone(),
        manifest_crc: cfg.manifest_crc,
        index_crc: cfg.index_crc,
    }))?;

    let begin = match recv_msg(&mut rx, hs_to, true, cfg, "begin")? {
        Message::Begin(b) => b,
        Message::Error(_) => return Err(()),
        _ => return Err(()),
    };
    let mut sampler = Sampler::new(begin.temp, begin.top_p, begin.seed);
    println!(
        "soso-llm: sesión {} iniciada (max_new={})",
        begin.session_id, begin.max_new
    );
    rx.send(&Message::Ack(AckPayload { pos: 0 }))?;

    rt.reset_sequence();
    let layer_start = cfg.layer_start;
    let layer_end = cfg.layer_end;

    loop {
        // Head en espera: keepalive detecta caída del head antes que step_timeout.
        match recv_msg(&mut rx, step_to, true, cfg, "step")? {
            Message::Step(step) => {
                if step.pos as usize != rt.pos {
                    let _ = rx.send(&Message::Error(pipeline::ErrorPayload {
                        code: pipeline::ERR_POS,
                        message: format!("pos desincronizada {} vs {}", step.pos, rt.pos),
                    }));
                    return Err(());
                }
                rt.set_hidden(&step.hidden)?;
                rt.forward_layers_range(layer_start, layer_end, source, par, &mut None)?;
                if is_tail && step.want_token != 0 {
                    let logits = rt.logits_par(source, par)?;
                    let token = sampler.sample(logits);
                    rx.send(&Message::Token(pipeline::TokenPayload {
                        pos: step.pos,
                        token,
                    }))?;
                } else {
                    rx.send(&Message::StepReply(StepReplyPayload {
                        pos: step.pos,
                        hidden: rt.hidden_slice().to_vec(),
                    }))?;
                }
                rt.advance_pos();
            }
            Message::Reset => {
                rt.reset_sequence();
                rx.send(&Message::Ack(AckPayload { pos: 0 }))?;
            }
            Message::Error(e) => {
                println!("soso-llm: head abortó sesión: {}", e.message);
                return Err(());
            }
            _ => return Err(()),
        }
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

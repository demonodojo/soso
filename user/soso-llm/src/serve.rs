//! Servidor HTTP guest con sesión residente (T16, C3–C4).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use core::sync::atomic::{AtomicBool, Ordering};

use libsoso::{println, sys};
use soso_abi as abi;
use soso_llm_api::{
    encode_api_error_json, encode_chat_completion_json, encode_stream_failure,
    encode_success_stream, format_response, format_sse_response_headers, prepare_chat_completion,
    text_delta_splits, AdmissionState, ApiError, CompletionPayload, EncodeError, FeedOutcome,
    HttpError, HttpReader, HttpRequest, PreparedChatCompletion,
};
use soso_llm_core::chat;
use soso_llm_core::conversation::{
    parse_assistant_output, render_messages, AssistantTurn, ChatError, ModelProfile, Role,
    ToolChoice,
};
use soso_llm_core::generation::{
    GenCheckpoint, GenerationObserver, GenerationReport, StopReason,
};

use crate::serve_poll::{write_bytes_or_cancel, ServePollEnv};
use soso_llm_core::plan::MemoryPlanConfig;
use soso_llm_core::tokenizer::Tokenizer;

use crate::ask::{atender_conexion_ask, leer_fichero, modelos, Conf, ASK_PORT};
use crate::generar_para_api;
use crate::net::TcpFd;
use crate::session::{
    perfil_api, preparar_sesion_api, PropietarioSesion, PoliticaModelo, resolver_modelo,
};

pub const HTTP_PORT_DEFAULT: u16 = 7422;

#[derive(Debug, Clone, PartialEq, Eq)]
enum GuestServiceError {
    Api(ApiError),
    Unauthorized,
    NotFound,
    MethodNotAllowed,
    Http(HttpError),
    Busy,
    Inferencia,
    Encode(EncodeError),
    Io,
}

struct ServeRuntime {
    token: String,
    profile: ModelProfile,
    backend: String,
    propietario: PropietarioSesion,
    conf: Conf,
    admission: AdmissionState,
    http_listener_fd: u64,
    /// Si ya se mandaron las cabeceras de esta respuesta. En streaming se
    /// mandan **antes** de generar, así que a partir de ahí el estado HTTP no
    /// se puede rectificar y un error tiene que ir dentro del flujo.
    cabeceras_enviadas: bool,
}

struct ServingObserver<'a> {
    cancel: &'a AtomicBool,
    env: ServePollEnv<'a>,
}

impl GenerationObserver for ServingObserver<'_> {
    fn cancel_at(&mut self, _: GenCheckpoint) -> bool {
        crate::serve_poll::poll_during_generation(&mut self.env, self.cancel);
        self.cancel.load(Ordering::Acquire)
    }
}

pub fn run(args: &[&str]) -> u8 {
    let model = flag(args, "--model");
    let port: u16 = flag(args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(HTTP_PORT_DEFAULT);
    let token_path = flag(args, "--token-file");

    let Some(model) = model else {
        uso_serve();
        return 1;
    };
    let Some(token_path) = token_path else {
        println!("serve: falta --token-file <ruta>");
        uso_serve();
        return 1;
    };
    if model.is_empty() {
        println!("serve: --model no puede estar vacío");
        return 1;
    }

    let catalog = modelos();
    if resolver_modelo(PoliticaModelo::ApiEstricta, &model, &catalog).is_none() {
        println!("serve: no hay ningún modelo «{model}» en /models");
        return 1;
    }

    let token = match leer_fichero(&token_path) {
        Some(t) => t.trim().to_string(),
        None => {
            println!("serve: no puedo leer el token en {token_path}");
            return 1;
        }
    };
    if token.is_empty() {
        println!("serve: el fichero de token está vacío");
        return 1;
    }

    libsoso::logln!("serve: cargando modelo {model}…");
    let sesion = match preparar_sesion_api(&model, false, MemoryPlanConfig::default()) {
        Ok(s) => s,
        Err(c) => {
            println!("serve: no pude cargar «{model}» (código {c})");
            return c;
        }
    };
    let profile = perfil_api(&sesion);
    let backend = if sesion.sys_gpu.is_some() {
        String::from("gpu")
    } else {
        String::from("cpu")
    };

    let ask_listener = match TcpFd::listen(ASK_PORT) {
        Ok(l) => l,
        Err(e) if e == -abi::EADDRINUSE => {
            println!(
                "serve: el puerto {ASK_PORT} (ask) ya está en uso; cierra el otro proceso o no uses askd aparte"
            );
            return 1;
        }
        Err(e) => {
            println!("serve: no pude escuchar ask en {ASK_PORT} (errno {e})");
            return 1;
        }
    };

    let http_listener = match TcpFd::listen(port) {
        Ok(l) => l,
        Err(e) => {
            println!("serve: no pude escuchar HTTP en {port} (errno {e})");
            return 1;
        }
    };

    libsoso::logln!(
        "serve: listo — ask={ASK_PORT} http={port} modelo={} backend={backend}",
        profile.id
    );

    let mut conf = Conf::default();
    conf.modelo = model.clone();

    let http_listener_fd = http_listener.fd;
    let mut rt = ServeRuntime {
        token,
        profile,
        backend,
        propietario: PropietarioSesion {
            sesion: Some(sesion),
            modelo: model,
        },
        conf,
        admission: AdmissionState::default(),
        http_listener_fd,
        cabeceras_enviadas: false,
    };

    loop {
        // Plazo corto, no `0`: `0` es «sin plazo» y duerme el proceso para
        // siempre en el primer `accept`, de forma que el listener de `ask`
        // nunca llegaba a atenderse mientras `serve` estuviera vivo.
        if let Ok(conn) = TcpFd::accept(&http_listener, ACCEPT_POLL_MS) {
            atender_http(&mut rt, conn);
            continue;
        }
        if let Ok(conn) = TcpFd::accept(&ask_listener, ACCEPT_POLL_MS) {
            atender_conexion_ask(&mut rt.propietario, &mut rt.conf, conn);
            continue;
        }
        let _ = sys::sleep_ms(20);
    }
}

fn atender_http(rt: &mut ServeRuntime, mut conn: TcpFd) {
    let req = match read_http_request(conn.fd) {
        Ok(r) => r,
        Err(e) => {
            let _ = write_bytes(conn.fd, &error_response(&e));
            return;
        }
    };
    // **Una petición, una respuesta.** En streaming las cabeceras salen antes
    // de generar, así que un fallo posterior —la validación de `tool_choice`,
    // por ejemplo— no puede contestarse con otra respuesta HTTP entera: el
    // cliente está leyendo un flujo y recibiría un `HTTP/1.1 400 …` como si
    // fueran datos. Eso es lo que vio la campaña de T14 en el caso Q07.
    rt.cabeceras_enviadas = false;
    match dispatch(rt, req, &mut conn) {
        Ok(resp) if !resp.is_empty() => {
            let _ = write_bytes(conn.fd, &resp);
        }
        Ok(_) => {}
        Err(e) if rt.cabeceras_enviadas => {
            // El error viaja **dentro** del flujo y se cierra con `[DONE]`,
            // con el mismo codificador que ya usa la cancelación.
            for ev in encode_stream_failure(&mensaje_error(&e), codigo_error(&e)) {
                let _ = write_bytes(conn.fd, &ev);
            }
        }
        Err(e) => {
            let _ = write_bytes(conn.fd, &error_response(&e));
        }
    }
}

/// Mensaje y código del error, para reportarlo dentro de un flujo ya abierto.
fn mensaje_error(err: &GuestServiceError) -> String {
    match err {
        GuestServiceError::Api(e) => e.message(),
        GuestServiceError::Unauthorized => String::from("autenticacion invalida"),
        GuestServiceError::NotFound => String::from("ruta desconocida"),
        GuestServiceError::MethodNotAllowed => String::from("metodo no permitido"),
        GuestServiceError::Busy => String::from("generacion en curso"),
        GuestServiceError::Inferencia => String::from("inferencia fallo"),
        _ => String::from("error interno"),
    }
}

fn codigo_error(err: &GuestServiceError) -> &'static str {
    match err {
        GuestServiceError::Api(e) => e.code(),
        GuestServiceError::Unauthorized => "invalid_api_key",
        GuestServiceError::NotFound => "not_found",
        GuestServiceError::MethodNotAllowed => "method_not_allowed",
        GuestServiceError::Busy => "busy",
        _ => "internal",
    }
}

fn dispatch(
    rt: &mut ServeRuntime,
    req: HttpRequest,
    conn: &mut TcpFd,
) -> Result<Vec<u8>, GuestServiceError> {
    if !check_bearer(&req, &rt.token) {
        return Ok(unauthorized_body());
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/health") => Ok(health_body("ready", &rt.profile.id, &rt.backend)),
        ("GET", "/v1/models") => Ok(models_body(&rt.profile.id)),
        ("POST", "/v1/chat/completions") => post_chat(rt, req, conn),
        (_, "/v1/chat/completions") => Err(GuestServiceError::MethodNotAllowed),
        _ => Err(GuestServiceError::NotFound),
    }
}

fn post_chat(
    rt: &mut ServeRuntime,
    req: HttpRequest,
    conn: &mut TcpFd,
) -> Result<Vec<u8>, GuestServiceError> {
    if rt.admission.begin_generation().is_err() {
        return Ok(busy_body());
    }
    let out = post_chat_inner(rt, req, conn);
    if let Some(ses) = rt.propietario.sesion.as_mut() {
        ses.reset_peticion();
        ses.bundle.rt.layer_hook = None;
        ses.bundle.rt.layer_enter_hook = None;
    }
    rt.admission.end_generation();
    out
}

fn post_chat_inner(
    rt: &mut ServeRuntime,
    req: HttpRequest,
    conn: &mut TcpFd,
) -> Result<Vec<u8>, GuestServiceError> {
    let body = core::str::from_utf8(&req.body).map_err(|_| {
        GuestServiceError::Api(ApiError::PeticionInvalida {
            motivo: String::from("cuerpo no UTF-8"),
        })
    })?;

    let profile = rt.profile.clone();
    let (prepared, prompt_ids) = {
        let sesion = rt
            .propietario
            .sesion
            .as_ref()
            .ok_or(GuestServiceError::Inferencia)?;
        let tokenizer = &sesion.bundle.tokenizer;
        let template = sesion.bundle.rt.manifest.chat_template.as_str();
        let prepared = prepare_chat_completion(body, &profile, tokenizer)
            .map_err(GuestServiceError::Api)?;
        let prompt_ids = prompt_ids_for_api(&prepared, &profile, tokenizer, template)
            .map_err(GuestServiceError::Api)?;
        (prepared, prompt_ids)
    };

    if prepared.stream {
        return post_chat_stream(rt, conn, prepared, prompt_ids, profile);
    }

    let (token_ids, report) = run_generation(rt, conn.fd, &prompt_ids, &prepared, &profile)?;
    if report.stop == StopReason::Cancelled {
        return Err(GuestServiceError::Inferencia);
    }

    // `generate_*` devuelve la secuencia **entera**: prompt + generación. Sin
    // recortarla, el `content` de la respuesta traía la plantilla renderizada
    // completa —`<|im_start|>system … <|im_start|>assistant\nLISTO`— mientras
    // `usage.completion_tokens` decía 2. El texto y el consumo se contradecían,
    // y cualquiera que leyera el `content` recibía el prompt de vuelta.
    let token_ids = solo_generados(&token_ids, prompt_ids.len());

    let generated_text = rt
        .propietario
        .sesion
        .as_ref()
        .ok_or(GuestServiceError::Inferencia)?
        .bundle
        .tokenizer
        .decode(&token_ids);
    let turn = parse_assistant_output(&prepared.input, &generated_text, 1)
        .map_err(|e| GuestServiceError::Api(e.into()))?;
    validate_tool_choice(&prepared, &turn)?;

    let payload = completion_payload(&prepared, &profile, turn, report);
    let json = encode_chat_completion_json(&payload).map_err(GuestServiceError::Encode)?;
    Ok(format_response(
        200,
        &[("Content-Type", "application/json")],
        &json,
    ))
}

/// Espera de cada `accept` del bucle principal. Corta para alternar entre los
/// dos listeners sin quemar CPU.
const ACCEPT_POLL_MS: u64 = 10;

fn run_generation(
    rt: &mut ServeRuntime,
    active_fd: u64,
    prompt_ids: &[u32],
    prepared: &PreparedChatCompletion,
    profile: &ModelProfile,
) -> Result<(Vec<u32>, GenerationReport), GuestServiceError> {
    let cancel = AtomicBool::new(false);
    let mut observer = ServingObserver {
        cancel: &cancel,
        env: ServePollEnv {
            http_listener_fd: rt.http_listener_fd,
            active_fd,
            admission: &mut rt.admission,
            token: &rt.token,
            model: &rt.profile.id,
            backend: &rt.backend,
        },
    };
    let sesion = rt
        .propietario
        .sesion
        .as_mut()
        .ok_or(GuestServiceError::Inferencia)?;
    generar_para_api(sesion, prompt_ids, prepared, profile, &mut observer)
        .map_err(|_| GuestServiceError::Inferencia)
}

fn post_chat_stream(
    rt: &mut ServeRuntime,
    conn: &mut TcpFd,
    prepared: PreparedChatCompletion,
    prompt_ids: Vec<u32>,
    profile: ModelProfile,
) -> Result<Vec<u8>, GuestServiceError> {
    let cancel = AtomicBool::new(false);
    write_bytes_or_cancel(conn.fd, &format_sse_response_headers(200), &cancel)
        .map_err(|_| GuestServiceError::Io)?;
    // A partir de aquí el estado HTTP ya está dicho y no se puede rectificar.
    rt.cabeceras_enviadas = true;

    let (token_ids, report) = {
        let mut observer = ServingObserver {
            cancel: &cancel,
            env: ServePollEnv {
                http_listener_fd: rt.http_listener_fd,
                active_fd: conn.fd,
                admission: &mut rt.admission,
                token: &rt.token,
                model: &rt.profile.id,
                backend: &rt.backend,
            },
        };
        let sesion = rt
            .propietario
            .sesion
            .as_mut()
            .ok_or(GuestServiceError::Inferencia)?;
        generar_para_api(sesion, &prompt_ids, &prepared, &profile, &mut observer)
            .map_err(|_| GuestServiceError::Inferencia)?
    };
    let token_ids = solo_generados(&token_ids, prompt_ids.len());

    if report.stop == StopReason::Cancelled || cancel.load(Ordering::Acquire) {
        let events = encode_stream_failure("generacion cancelada", "cancelled");
        for e in events {
            let _ = write_bytes_or_cancel(conn.fd, &e, &cancel);
        }
        return Ok(Vec::new());
    }

    let generated_text = rt
        .propietario
        .sesion
        .as_ref()
        .ok_or(GuestServiceError::Inferencia)?
        .bundle
        .tokenizer
        .decode(&token_ids);
    let turn = parse_assistant_output(&prepared.input, &generated_text, 1)
        .map_err(|e| GuestServiceError::Api(e.into()))?;
    validate_tool_choice(&prepared, &turn)?;

    let payload = completion_payload(&prepared, &profile, turn, report);
    let text = payload.content.as_deref().unwrap_or("");
    let splits = if text.is_empty() {
        Vec::new()
    } else {
        text_delta_splits(text, &[text.len()])
    };
    let events = encode_success_stream(&payload, &splits, prepared.include_usage)
        .map_err(GuestServiceError::Encode)?;
    for e in events {
        write_bytes_or_cancel(conn.fd, &e, &cancel).map_err(|_| GuestServiceError::Io)?;
    }
    Ok(Vec::new())
}

fn prompt_ids_for_api(
    prepared: &PreparedChatCompletion,
    profile: &ModelProfile,
    tokenizer: &Tokenizer,
    template: &str,
) -> Result<Vec<u32>, ApiError> {
    match render_messages(&prepared.input, profile, tokenizer) {
        Ok(ids) => Ok(ids),
        Err(ChatError::FamiliaNoImplementada { .. }) => {
            if prepared.input.messages.len() != 1 {
                return Err(ApiError::Dominio(ChatError::FamiliaNoImplementada {
                    family: profile.family.clone(),
                }));
            }
            let msg = &prepared.input.messages[0];
            if msg.role != Role::User {
                return Err(ApiError::Dominio(ChatError::FamiliaNoImplementada {
                    family: profile.family.clone(),
                }));
            }
            let texto = msg.content.as_deref().unwrap_or("");
            Ok(chat::render(template, texto, tokenizer))
        }
        Err(e) => Err(e.into()),
    }
}

fn validate_tool_choice(
    prepared: &PreparedChatCompletion,
    turn: &AssistantTurn,
) -> Result<(), GuestServiceError> {
    match prepared.input.tool_choice {
        ToolChoice::Required | ToolChoice::Named(_) if turn.tool_call.is_none() => {
            Err(GuestServiceError::Api(ApiError::Dominio(
                ChatError::SeleccionInvalida {
                    motivo: String::from("tool_choice exige una llamada válida"),
                },
            )))
        }
        _ => Ok(()),
    }
}

fn read_http_request(fd: u64) -> Result<HttpRequest, GuestServiceError> {
    let mut reader = HttpReader::new();
    let mut buf = [0u8; 8192];
    let deadline = sys::uptime_ms().saturating_add(120_000);
    loop {
        if sys::uptime_ms() > deadline {
            return Err(GuestServiceError::Http(HttpError::IncompleteBody));
        }
        let n = sys::read_timeout(fd, &mut buf, crate::net::READ_CHUNK_MS);
        if n == -(abi::EAGAIN as i64) {
            continue;
        }
        if n <= 0 {
            return match reader.signal_eof() {
                Ok(FeedOutcome::Complete(req)) => Ok(req),
                _ => Err(GuestServiceError::Http(HttpError::IncompleteBody)),
            };
        }
        match reader
            .feed(&buf[..n as usize])
            .map_err(GuestServiceError::Http)?
        {
            FeedOutcome::Complete(req) => return Ok(req),
            FeedOutcome::NeedMore => continue,
        }
    }
}

fn check_bearer(req: &HttpRequest, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("authorization") {
            let v = v.trim();
            if let Some(rest) = v.strip_prefix("Bearer ") {
                return rest.trim() == token;
            }
            if let Some(rest) = v.strip_prefix("bearer ") {
                return rest.trim() == token;
            }
        }
    }
    false
}

fn completion_payload(
    prepared: &PreparedChatCompletion,
    _profile: &ModelProfile,
    turn: AssistantTurn,
    report: GenerationReport,
) -> CompletionPayload {
    CompletionPayload {
        id: format!("chatcmpl-{}", unix_now_secs()),
        model: prepared.model_id.clone(),
        created: unix_now_secs(),
        content: turn.content,
        tool_calls: turn.tool_call.map(|c| vec![c]).unwrap_or_default(),
        report,
    }
}

fn unix_now_secs() -> u64 {
    let mut ts = abi::Timespec::default();
    if sys::clock_gettime(abi::CLOCK_REALTIME, &mut ts) == 0 {
        ts.tv_sec as u64
    } else {
        sys::uptime_ms().max(0) as u64 / 1000
    }
}

fn write_bytes(fd: u64, data: &[u8]) -> Result<(), ()> {
    sys::write_all(fd, data).map_err(|_| ())
}

fn health_body(status: &str, model: &str, backend: &str) -> Vec<u8> {
    let body = format!(
        r#"{{"status":"{status}","model":"{model}","backend":"{backend}"}}"#
    );
    format_response(200, &[("Content-Type", "application/json")], body.as_bytes())
}

fn models_body(model_id: &str) -> Vec<u8> {
    let body = format!(
        r#"{{"object":"list","data":[{{"id":"{model_id}","object":"model"}}]}}"#
    );
    format_response(200, &[("Content-Type", "application/json")], body.as_bytes())
}

fn unauthorized_body() -> Vec<u8> {
    let body = "{\"error\":{\"message\":\"autenticacion invalida\",\"type\":\"invalid_request_error\",\"code\":\"invalid_api_key\"}}";
    format_response(401, &[("Content-Type", "application/json")], body.as_bytes())
}

fn busy_body() -> Vec<u8> {
    let body = "{\"error\":{\"message\":\"generacion en curso\",\"type\":\"invalid_request_error\",\"code\":\"busy\"}}";
    format_response(429, &[("Content-Type", "application/json")], body.as_bytes())
}

/// Quita el prompt de la secuencia devuelta por el generador.
///
/// Se hace por longitud y no buscando un separador: los ids del prompt son
/// exactamente los que se le pasaron, y un separador podría aparecer también
/// en lo generado.
fn solo_generados(secuencia: &[u32], prompt_len: usize) -> Vec<u32> {
    if secuencia.len() <= prompt_len {
        return Vec::new();
    }
    secuencia[prompt_len..].to_vec()
}

fn error_response(err: &GuestServiceError) -> Vec<u8> {
    match err {
        GuestServiceError::Api(e) => format_response(
            e.status_code(),
            &[("Content-Type", "application/json")],
            &encode_api_error_json(e),
        ),
        GuestServiceError::Unauthorized => unauthorized_body(),
        GuestServiceError::NotFound => format_response(
            404,
            &[("Content-Type", "application/json")],
            br#"{"error":{"message":"ruta desconocida","type":"not_found_error","code":"not_found"}}"#,
        ),
        GuestServiceError::MethodNotAllowed => format_response(
            405,
            &[("Content-Type", "application/json")],
            br#"{"error":{"message":"metodo no permitido","type":"invalid_request_error","code":"method_not_allowed"}}"#,
        ),
        GuestServiceError::Busy => busy_body(),
        GuestServiceError::Inferencia => format_response(
            500,
            &[("Content-Type", "application/json")],
            br#"{"error":{"message":"inferencia fallo","type":"server_error","code":"inference_error"}}"#,
        ),
        GuestServiceError::Encode(_) | GuestServiceError::Http(_) | GuestServiceError::Io => {
            format_response(
                500,
                &[("Content-Type", "application/json")],
                br#"{"error":{"message":"error interno","type":"internal_error","code":"internal"}}"#,
            )
        }
    }
}

fn flag(args: &[&str], name: &str) -> Option<String> {
    args.iter()
        .position(|&p| p == name)
        .and_then(|i| args.get(i + 1))
        .map(|&v| String::from(v))
}

fn uso_serve() {
    println!("uso:");
    println!("  soso-llm serve --model <nombre> --port <puerto> --token-file <ruta>");
    println!("    (puerto HTTP por defecto {HTTP_PORT_DEFAULT}; ask en {ASK_PORT})");
    println!("    Sondeo cooperativo T17; producción: pendiente de T14 go.");
}

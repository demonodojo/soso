//! T10 — parseo y preparación de peticiones Chat Completions.

use std::path::{Path, PathBuf};

use soso_llm_api::{
    parse_chat_completions, prepare_chat_completion, validate_wire_policy, wire_to_chat_input,
    ApiError, WireChatCompletion,
};
use soso_llm_core::conversation::{ModelProfile, Role, MAX_MENSAJES};
use soso_llm_core::tokenizer::Tokenizer;

fn raiz_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("raíz del repo")
}

fn tokenizer_qwen() -> Option<Tokenizer> {
    let ruta = raiz_repo().join("target/qwen2.5-coder-3b-model/tokenizer.som");
    let datos = std::fs::read(ruta).ok()?;
    let tok = Tokenizer::parse(&datos).ok()?;
    tok.tiene_vocabulario().then_some(tok)
}

fn perfil_qwen() -> ModelProfile {
    ModelProfile {
        id: String::from("soso-coder"),
        directory: String::from("target/qwen2.5-coder-3b-model"),
        family: String::from("qwen2"),
        weights_sha256: String::from("0"),
        tokenizer_sha256: String::from("0"),
        template_sha256: String::from("0"),
        context_tokens: 32_768,
        max_output_tokens: 2048,
        stop_token_ids: vec![151645, 151643],
    }
}

fn base_request(extra: &str) -> String {
    format!(
        r#"{{
  "model": "soso-coder",
  "messages": [{{"role": "user", "content": "hola"}}],
  {extra}
}}"#
    )
}

#[test]
fn modelo_ausente_es_400() {
    let body = r#"{"messages":[{"role":"user","content":"x"}]}"#;
    let wire = parse_chat_completions(body).unwrap();
    let err = validate_wire_policy(&wire).unwrap_err();
    assert_eq!(err.status_code(), 400);
}

#[test]
fn alias_max_tokens_contradictorios() {
    let body = base_request(r#""max_tokens": 10, "max_completion_tokens": 20"#);
    let wire = parse_chat_completions(&body).unwrap();
    let err = validate_wire_policy(&wire).unwrap_err();
    assert!(matches!(err, ApiError::AliasContradictorio { .. }));
    assert_eq!(err.status_code(), 400);
}

#[test]
fn alias_max_tokens_iguales_ok() {
    let body = base_request(r#""max_tokens": 10, "max_completion_tokens": 10"#);
    let wire = parse_chat_completions(&body).unwrap();
    validate_wire_policy(&wire).unwrap();
}

#[test]
fn temperatura_no_finita_rechazada() {
    let body = base_request(r#""temperature": 0.5"#);
    let mut wire: WireChatCompletion = parse_chat_completions(&body).unwrap();
    wire.temperature = Some(f64::NAN);
    let err = validate_wire_policy(&wire).unwrap_err();
    assert!(matches!(err, ApiError::ParametroNoFinito { .. }));
}

#[test]
fn partes_texto_ok() {
    let body = r#"{
      "model": "soso-coder",
      "messages": [{
        "role": "user",
        "content": [
          {"type": "text", "text": "a"},
          {"type": "text", "text": "b"}
        ]
      }]
    }"#;
    let wire = parse_chat_completions(body).unwrap();
    let input = wire_to_chat_input(&wire).unwrap();
    assert_eq!(input.messages[0].content.as_deref(), Some("a\nb"));
}

#[test]
fn imagen_rechazada_422() {
    let body = r#"{
      "model": "soso-coder",
      "messages": [{
        "role": "user",
        "content": [{"type": "image_url", "image_url": {"url": "x"}}]
      }]
    }"#;
    let wire = parse_chat_completions(body).unwrap();
    let err = wire_to_chat_input(&wire).unwrap_err();
    assert!(matches!(err, ApiError::ModalidadNoSoportada { .. }));
    assert_eq!(err.status_code(), 422);
}

#[test]
fn limite_mensajes_exacto_y_mas_uno() {
    let mut msgs = String::from("[");
    for i in 0..MAX_MENSAJES {
        if i > 0 {
            msgs.push(',');
        }
        msgs.push_str(&format!(r#"{{"role":"user","content":"{i}"}}"#));
    }
    msgs.push(']');
    let body = format!(
        r#"{{"model":"soso-coder","messages":{msgs}}}"#
    );
    let wire = parse_chat_completions(&body).unwrap();
    let input = wire_to_chat_input(&wire).unwrap();
    soso_llm_core::conversation::validate_input(&input).unwrap();

    let mut msgs65 = msgs.clone();
    msgs65.insert(msgs65.len() - 1, ',');
    msgs65.insert_str(
        msgs65.len() - 1,
        r#"{"role":"user","content":"overflow"}"#,
    );
    let body65 = format!(r#"{{"model":"soso-coder","messages":{msgs65}}}"#);
    let wire65 = parse_chat_completions(&body65).unwrap();
    let input65 = wire_to_chat_input(&wire65).unwrap();
    let err = soso_llm_core::conversation::validate_input(&input65).unwrap_err();
    let api: ApiError = err.into();
    assert_eq!(api.status_code(), 400);
}

#[test]
fn q04_peticion_wire_a_dominio() {
    let path = raiz_repo().join("tests/self-improvement/cases/visible/Q04/peticion.json");
    let body = std::fs::read_to_string(path).expect("Q04 peticion");
    let wire = parse_chat_completions(&body).unwrap();
    validate_wire_policy(&wire).unwrap();
    let input = wire_to_chat_input(&wire).unwrap();
    soso_llm_core::conversation::validate_input(&input).unwrap();
    assert_eq!(input.messages.len(), 3);
    assert!(input.messages[1].content.is_none());
    assert_eq!(input.messages[1].tool_calls.len(), 1);
    assert_eq!(input.messages[2].role, Role::Tool);
}

#[test]
fn q04_prepare_si_hay_tokenizer() {
    let Some(tokenizer) = tokenizer_qwen() else {
        return;
    };
    let path = raiz_repo().join("tests/self-improvement/cases/visible/Q04/peticion.json");
    let body = std::fs::read_to_string(path).expect("Q04 peticion");
    let mut profile = perfil_qwen();
    profile.id = String::from("soso-coder");
    // Q04 usa otro id de modelo en el JSON; alinear con perfil para prepare
    let body = body.replace(
        "qwen2.5-coder-3b-instruct",
        "soso-coder",
    );
    let prep = prepare_chat_completion(&body, &profile, &tokenizer).unwrap();
    assert!(prep.prompt_tokens > 0);
    assert_eq!(prep.max_new_tokens, 32);
    assert!(!prep.stream);
}

#[test]
fn modelo_distinto_perfil_404() {
    let Some(tokenizer) = tokenizer_qwen() else {
        return;
    };
    let body = r#"{
      "model": "otro-modelo",
      "messages": [{"role": "user", "content": "hola"}]
    }"#;
    let profile = perfil_qwen();
    let err = prepare_chat_completion(&body, &profile, &tokenizer).unwrap_err();
    assert!(matches!(err, ApiError::ModeloDesconocido { .. }));
    assert_eq!(err.status_code(), 404);
}

#[test]
fn contexto_excedido_422() {
    let Some(tokenizer) = tokenizer_qwen() else {
        return;
    };
    let texto = "palabra ".repeat(4000);
    let body = format!(
        r#"{{
      "model": "soso-coder",
      "messages": [{{"role": "user", "content": {texto:?}}}],
      "max_tokens": 5000
    }}"#
    );
    let mut profile = perfil_qwen();
    profile.context_tokens = 64;
    profile.max_output_tokens = 32;
    let err = prepare_chat_completion(&body, &profile, &tokenizer).unwrap_err();
    assert_eq!(err.status_code(), 422);
}

#[test]
fn campo_desconocido_raiz_rechazado() {
    let body = base_request(r#""extra_field": true"#);
    assert!(parse_chat_completions(&body).is_err());
}

#[test]
fn error_body_forma_openai() {
    let err = ApiError::ModeloDesconocido {
        model: String::from("otro"),
    };
    let v = err.to_error_body();
    assert!(v.get("error").unwrap().get("message").is_some());
    assert_eq!(err.status_code(), 404);
}

/// T60: cada causa de rechazo tiene su código. `unknown_tool` caía en el cajón
/// de sastre `invalid_request`, y la campaña de T14 (caso Q08) lo vio.
#[test]
fn codigos_de_error_distinguen_la_causa() {
    use soso_llm_api::ApiError;
    use soso_llm_core::conversation::ChatError;

    let desconocida = ApiError::Dominio(ChatError::HerramientaDesconocida {
        nombre: String::from("no_existe"),
    });
    assert_eq!(desconocida.code(), "unknown_tool");
    assert_eq!(desconocida.status_code(), 400);
    assert_eq!(desconocida.error_type(), "invalid_request_error");

    // Los vecinos no se mueven: un JSON malo y un contexto pasado siguen
    // diciendo lo suyo.
    let json_malo = ApiError::JsonInvalido {
        motivo: String::from("x"),
    };
    assert_eq!(json_malo.code(), "invalid_json");
    let contexto = ApiError::Dominio(ChatError::ContextoExcedido {
        necesarios: 40_000,
        disponibles: 32_768,
    });
    assert_eq!(contexto.code(), "context_length_exceeded");
    assert_eq!(contexto.status_code(), 422);
}

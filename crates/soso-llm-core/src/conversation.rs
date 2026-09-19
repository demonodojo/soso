//! Tipos de conversación: lo que se le manda a un modelo de chat y lo que se
//! espera de él (ficha T04, contrato C1).
//!
//! Aquí no hay HTTP, ni sockets, ni syscalls. Son los tipos del **dominio**: el
//! adaptador de red los traduce al formato del proveedor, y no al revés. Esa
//! dirección importa porque el mismo dominio tiene que valer para el servidor
//! host y para el guest, y porque el formato de la API puede cambiar sin que
//! cambie lo que una conversación *es*.
//!
//! Dos decisiones que se notan en todo el módulo:
//!
//! - **Texto ausente no es texto vacío.** `content: Option<String>`: un
//!   asistente que solo llama a una herramienta manda `None`, y un mensaje con
//!   `Some("")` es otra cosa. Colapsar los dos casos es lo que hace que un
//!   historial deje de poder reconstruirse.
//! - **Los argumentos de una llamada viajan como texto.** El modelo escribe
//!   JSON dentro de su respuesta; ese JSON puede llegar troceado o mal formado,
//!   y convertirlo a `Value` demasiado pronto pierde la diferencia entre «no ha
//!   terminado de escribirlo» y «escribió algo que no es JSON».
//!
//! La serialización que se deriva aquí es para **fixtures y persistencia**. No
//! es el formato Chat Completions: eso lo construye `soso-llm-api` (T10).

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Validación del historial y de los esquemas de herramientas (T05).
pub mod validate;
/// Render de historial por familia de modelo (T06).
pub mod render;
/// Parse de `<tool_call>` en salida del modelo (T07).
pub mod tools;

pub use render::render_messages;
pub use tools::{parse_assistant_output, AssistantTurn, ToolCallParser};

pub use validate::{
    validate_assistant_turn, validate_input, validate_parsed_tool_call, MAX_ARGUMENTOS_BYTES,
    MAX_HERRAMIENTAS, MAX_MENSAJES, MAX_PROFUNDIDAD_ESQUEMA,
};

/// Quién habla en un turno.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// Resultado de una herramienta, asociado a la llamada que lo pidió.
    Tool,
}

impl Role {
    pub fn como_texto(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// Una llamada a herramienta emitida por el modelo.
///
/// `arguments` es **texto**, no `Value`: es lo que el modelo escribió. Quien lo
/// vaya a ejecutar lo valida antes (T07); mientras tanto se conserva tal cual,
/// que es lo que permite distinguir un JSON a medias de uno inválido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl ToolCall {
    pub fn nueva(id: impl Into<String>, name: impl Into<String>, arguments: impl Into<String>) -> Self {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    /// Los argumentos ya interpretados, si son un objeto JSON completo.
    pub fn argumentos(&self) -> Result<Value, ChatError> {
        let valor: Value = serde_json::from_str(&self.arguments)
            .map_err(|_| ChatError::ArgumentosInvalidos {
                llamada: self.id.clone(),
            })?;
        if !valor.is_object() {
            return Err(ChatError::ArgumentosInvalidos {
                llamada: self.id.clone(),
            });
        }
        Ok(valor)
    }
}

/// Un turno de la conversación.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// `None` = el turno no lleva texto. Distinto de `Some(String::new())`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Llamadas emitidas en este turno. Vacío en todos los roles menos
    /// `assistant`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// A qué llamada responde este turno. Solo en `tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn texto(role: Role, content: impl Into<String>) -> Self {
        Message {
            role,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::texto(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::texto(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::texto(Role::Assistant, content)
    }

    /// Turno de asistente que solo llama a herramientas: sin texto.
    pub fn llamadas(calls: Vec<ToolCall>) -> Self {
        Message {
            role: Role::Assistant,
            content: None,
            tool_calls: calls,
            tool_call_id: None,
        }
    }

    /// Resultado de una herramienta, atado a la llamada que lo pidió.
    pub fn resultado(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Message {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    /// ¿Tiene algo que decir? Un turno sin texto **ni** llamadas no aporta nada
    /// al historial, y eso es distinto de tener texto vacío.
    pub fn vacio(&self) -> bool {
        self.content.is_none() && self.tool_calls.is_empty()
    }
}

/// Una herramienta que el modelo puede llamar.
///
/// `parameters` es un `Value` porque es un JSON Schema: se transporta y se
/// compara, no se interpreta aquí. El `"type": "function"` del formato de la
/// API pertenece al cable, no al dominio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
}

impl ToolDefinition {
    pub fn nueva(name: impl Into<String>, description: Option<String>, parameters: Value) -> Self {
        ToolDefinition {
            name: name.into(),
            description,
            parameters,
        }
    }
}

/// Qué se le permite o se le exige al modelo en este turno.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoice {
    /// Texto o llamada, lo que el modelo decida.
    Auto,
    /// Texto; las herramientas quedan declaradas pero no se usan.
    None,
    /// Una llamada, de la herramienta que sea.
    Required,
    /// Una llamada de esta herramienta en concreto.
    Named(String),
}

impl Default for ToolChoice {
    fn default() -> Self {
        ToolChoice::Auto
    }
}

/// Lo que se manda a generar.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChatInput {
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub tool_choice: ToolChoice,
}

impl ChatInput {
    pub fn nuevo(messages: Vec<Message>) -> Self {
        ChatInput {
            messages,
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
        }
    }

    pub fn con_herramientas(mut self, tools: Vec<ToolDefinition>, choice: ToolChoice) -> Self {
        self.tools = tools;
        self.tool_choice = choice;
        self
    }

    pub fn herramienta(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.iter().find(|t| t.name == name)
    }
}

/// El modelo que se va a usar, identificado de forma estable.
///
/// El id **no** es el nombre del directorio: un directorio se renombra y una
/// campaña deja de ser comparable. Los hashes son los que fija el
/// `model-lock.json` de T03, que es metadato de la evaluación y viaja aparte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Identificador estable con el que se le habla (`soso-coder`).
    pub id: String,
    /// Dónde están los pesos de verdad.
    pub directory: String,
    /// Familia de plantilla y tokenizer (`qwen2`…).
    pub family: String,
    pub weights_sha256: String,
    pub tokenizer_sha256: String,
    pub template_sha256: String,
    /// Contexto **comprobado**, no el que anuncia el fabricante.
    pub context_tokens: u32,
    /// Techo de tokens por respuesta.
    pub max_output_tokens: u32,
    /// Todos los ids que paran la generación. Suelen ser varios.
    pub stop_token_ids: Vec<u32>,
}

impl ModelProfile {
    /// ¿Cabe una petición de este tamaño dejando sitio para la respuesta?
    ///
    /// El límite es `render completo + reserva <= contexto` (C3). No se trunca
    /// el historial para que quepa: eso esconde el desbordamiento.
    pub fn cabe(&self, prompt_tokens: u32, reserva_salida: u32) -> bool {
        prompt_tokens.saturating_add(reserva_salida) <= self.context_tokens
    }

    pub fn es_parada(&self, token: u32) -> bool {
        self.stop_token_ids.contains(&token)
    }
}

/// Lo que puede ir mal en una conversación, antes de generar.
///
/// Son errores distintos a propósito: el adaptador HTTP los mapea a códigos
/// distintos (C3), y mezclarlos en un `String` obliga a adivinar después.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatError {
    /// Historial vacío o con un turno que no aporta nada.
    HistorialInvalido { motivo: String },
    /// Un `tool` sin `tool_call_id`, o que apunta a una llamada que no existe.
    /// El id viaja vacío cuando el mensaje no traía ninguno.
    ResultadoHuerfano { tool_call_id: String },
    /// Dos resultados para la misma llamada.
    ResultadoDuplicado { tool_call_id: String },
    /// Dos llamadas con el mismo id en el historial.
    LlamadaDuplicada { llamada: String },
    /// Una llamada anterior se quedó sin resultado: no se puede generar encima.
    LlamadaPendiente { llamada: String },
    /// Dos herramientas declaradas con el mismo nombre.
    HerramientaDuplicada { nombre: String },
    /// `tool_choice` no se puede satisfacer con lo declarado.
    SeleccionInvalida { motivo: String },
    /// El esquema de una herramienta usa algo fuera del subconjunto admitido.
    /// `ruta` es un JSON Pointer dentro del esquema.
    EsquemaNoSoportado {
        herramienta: String,
        ruta: String,
        motivo: String,
    },
    /// Un argumento concreto no cumple el esquema. `ruta` es un JSON Pointer
    /// dentro del objeto de argumentos.
    ArgumentoInvalido {
        llamada: String,
        ruta: String,
        motivo: String,
    },
    /// Un límite de C3: mensajes, herramientas o tamaño de argumentos.
    LimiteExcedido {
        que: String,
        maximo: u32,
        recibido: u32,
    },
    /// `tool_choice` nombra una herramienta que no está declarada.
    HerramientaDesconocida { nombre: String },
    /// Los argumentos de una llamada no son un objeto JSON completo.
    ArgumentosInvalidos { llamada: String },
    /// El render más la reserva de salida no caben en el contexto.
    ContextoExcedido { necesarios: u32, disponibles: u32 },
    /// La familia del perfil no tiene renderer en esta entrega.
    FamiliaNoImplementada { family: String },
    /// El tokenizer no puede representar el texto renderizado.
    TokenizacionInvalida { motivo: String },
}

/// La raíz de un JSON Pointer es la cadena vacía; en un mensaje se lee mal.
fn ruta_legible(ruta: &str) -> &str {
    if ruta.is_empty() { "(raíz)" } else { ruta }
}

impl core::fmt::Display for ChatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ChatError::HistorialInvalido { motivo } => write!(f, "historial inválido: {motivo}"),
            ChatError::ResultadoHuerfano { tool_call_id } => write!(
                f,
                "resultado de herramienta sin llamada que lo pidiera: {tool_call_id}"
            ),
            ChatError::ResultadoDuplicado { tool_call_id } => {
                write!(f, "dos resultados para la misma llamada: {tool_call_id}")
            }
            ChatError::LlamadaDuplicada { llamada } => {
                write!(f, "dos llamadas con el mismo id: {llamada}")
            }
            ChatError::LlamadaPendiente { llamada } => {
                write!(f, "la llamada {llamada} sigue sin resultado")
            }
            ChatError::HerramientaDuplicada { nombre } => {
                write!(f, "dos herramientas declaradas como {nombre}")
            }
            ChatError::SeleccionInvalida { motivo } => {
                write!(f, "tool_choice inválido: {motivo}")
            }
            ChatError::EsquemaNoSoportado {
                herramienta,
                ruta,
                motivo,
            } => write!(
                f,
                "esquema de {herramienta} en {}: {motivo}",
                ruta_legible(ruta)
            ),
            ChatError::ArgumentoInvalido {
                llamada,
                ruta,
                motivo,
            } => write!(
                f,
                "argumentos de {llamada} en {}: {motivo}",
                ruta_legible(ruta)
            ),
            ChatError::LimiteExcedido {
                que,
                maximo,
                recibido,
            } => write!(f, "{que}: {recibido} supera el máximo de {maximo}"),
            ChatError::HerramientaDesconocida { nombre } => {
                write!(f, "herramienta no declarada: {nombre}")
            }
            ChatError::ArgumentosInvalidos { llamada } => {
                write!(f, "argumentos que no son un objeto JSON: {llamada}")
            }
            ChatError::ContextoExcedido {
                necesarios,
                disponibles,
            } => write!(
                f,
                "el render necesita {necesarios} tokens y hay {disponibles}"
            ),
            ChatError::FamiliaNoImplementada { family } => {
                write!(f, "familia de chat no implementada: {family}")
            }
            ChatError::TokenizacionInvalida { motivo } => {
                write!(f, "tokenización inválida: {motivo}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ChatError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn texto_ausente_y_texto_vacio_son_distintos() {
        let sin_texto = Message::llamadas(vec![ToolCall::nueva("c1", "f", "{}")]);
        let vacio = Message::texto(Role::Assistant, "");
        assert_eq!(sin_texto.content, None);
        assert_eq!(vacio.content, Some(String::new()));
        assert_ne!(sin_texto, vacio);
        // Y el JSON lo conserva: el que no tiene texto no escribe el campo.
        let a = serde_json::to_string(&sin_texto).unwrap();
        let b = serde_json::to_string(&vacio).unwrap();
        assert!(!a.contains("content"), "{a}");
        assert!(b.contains("\"content\":\"\""), "{b}");
    }

    #[test]
    fn un_turno_sin_texto_ni_llamadas_esta_vacio() {
        let m = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
        };
        assert!(m.vacio());
        assert!(!Message::assistant("").vacio(), "texto vacío sigue siendo texto");
    }

    #[test]
    fn los_argumentos_se_conservan_como_texto() {
        let parcial = ToolCall::nueva("c1", "leer", "{\"ruta\": \"a.rs\",");
        // Se guarda tal cual: es lo que el modelo escribió.
        assert_eq!(parcial.arguments, "{\"ruta\": \"a.rs\",");
        assert_eq!(
            parcial.argumentos(),
            Err(ChatError::ArgumentosInvalidos {
                llamada: String::from("c1")
            })
        );
        let entera = ToolCall::nueva("c2", "leer", "{\"ruta\":\"a.rs\",\"lineas\":20}");
        let v = entera.argumentos().expect("objeto completo");
        assert_eq!(v["lineas"], 20);
        // Una lista JSON no es un objeto de argumentos.
        assert!(ToolCall::nueva("c3", "f", "[1,2]").argumentos().is_err());
    }

    #[test]
    fn ida_y_vuelta_de_cada_rol() {
        let casos = vec![
            Message::system("eres breve"),
            Message::user("hola"),
            Message::assistant("qué tal"),
            Message::llamadas(vec![ToolCall::nueva("c1", "hora", "{\"zona\":\"local\"}")]),
            Message::resultado("c1", "13:45"),
        ];
        for m in casos {
            let json = serde_json::to_string(&m).unwrap();
            let vuelta: Message = serde_json::from_str(&json).unwrap();
            assert_eq!(m, vuelta, "{json}");
        }
    }

    #[test]
    fn el_unicode_sobrevive_a_la_ida_y_vuelta() {
        let m = Message::user("café ☕ para el señor Ñoño");
        let vuelta: Message = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(vuelta.content.as_deref(), Some("café ☕ para el señor Ñoño"));
    }

    #[test]
    fn varias_llamadas_y_sus_resultados() {
        // El perfil inicial emite una por turno, pero el historial acepta los
        // turnos anteriores tal y como fueran.
        let turno = Message::llamadas(vec![
            ToolCall::nueva("c1", "leer", "{\"ruta\":\"a\"}"),
            ToolCall::nueva("c2", "leer", "{\"ruta\":\"b\"}"),
        ]);
        let entrada = ChatInput::nuevo(vec![
            Message::user("lee a y b"),
            turno.clone(),
            Message::resultado("c1", "contenido de a"),
            Message::resultado("c2", "contenido de b"),
        ]);
        let json = serde_json::to_string(&entrada).unwrap();
        let vuelta: ChatInput = serde_json::from_str(&json).unwrap();
        assert_eq!(entrada, vuelta);
        assert_eq!(vuelta.messages[1].tool_calls.len(), 2);
        assert_eq!(vuelta.messages[3].tool_call_id.as_deref(), Some("c2"));
    }

    #[test]
    fn la_seleccion_de_herramienta_va_y_vuelve() {
        for choice in [
            ToolChoice::Auto,
            ToolChoice::None,
            ToolChoice::Required,
            ToolChoice::Named(String::from("leer")),
        ] {
            let json = serde_json::to_string(&choice).unwrap();
            let vuelta: ToolChoice = serde_json::from_str(&json).unwrap();
            assert_eq!(choice, vuelta, "{json}");
        }
        assert_eq!(ToolChoice::default(), ToolChoice::Auto);
    }

    #[test]
    fn el_esquema_de_la_herramienta_se_transporta_entero() {
        let esquema = serde_json::json!({
            "type": "object",
            "properties": {"ruta": {"type": "string"}},
            "required": ["ruta"]
        });
        let t = ToolDefinition::nueva("leer", Some(String::from("Lee")), esquema.clone());
        let vuelta: ToolDefinition =
            serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(vuelta.parameters, esquema);
        // El `type: function` es del cable, no del dominio.
        assert!(!serde_json::to_string(&t).unwrap().contains("\"function\""));
    }

    #[test]
    fn el_perfil_decide_si_cabe_y_qué_para() {
        let p = ModelProfile {
            id: String::from("soso-coder"),
            directory: String::from("/models/qwen"),
            family: String::from("qwen2"),
            weights_sha256: String::from("0"),
            tokenizer_sha256: String::from("0"),
            template_sha256: String::from("0"),
            context_tokens: 100,
            max_output_tokens: 32,
            stop_token_ids: vec![151645, 151643],
        };
        assert!(p.cabe(60, 40));
        assert!(!p.cabe(61, 40));
        // Sin desbordar la aritmética.
        assert!(!p.cabe(u32::MAX, 1));
        assert!(p.es_parada(151643));
        assert!(!p.es_parada(13));
    }
}

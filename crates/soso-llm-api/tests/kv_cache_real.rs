//! Integración con modelo SOM sintético generado por mkmodel-soso.
#![cfg(feature = "std")]

use soso_llm_api::{load_cpu_backend, sha256_hex, ChatBackend, PreparedChatCompletion};
use soso_llm_core::conversation::{ChatInput, ModelProfile};
use soso_llm_core::generation::NoCancelObserver;
use std::path::PathBuf;

#[test]
#[ignore = "requiere SOSO_TEST_MODEL con modelo SOM generado"]
fn servicio_recupera_prefijo_tras_reinicio_y_recalcula_si_hay_corrupcion() {
    use std::os::unix::fs::PermissionsExt;
    let dir = PathBuf::from(std::env::var("SOSO_TEST_MODEL").expect("SOSO_TEST_MODEL"));
    let tmp = std::env::temp_dir().join(format!("soso-kv-real-{}", std::process::id()));
    std::fs::create_dir(&tmp).unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700)).unwrap();
    let perfil = ModelProfile {
        id: "tiny".into(), directory: dir.to_string_lossy().into_owned(), family: "qwen2".into(),
        weights_sha256: sha256_hex(&std::fs::read(dir.join("index.som")).unwrap()),
        tokenizer_sha256: sha256_hex(&std::fs::read(dir.join("tokenizer.som")).unwrap()),
        template_sha256: String::new(), context_tokens: 128, max_output_tokens: 8, stop_token_ids: vec![],
    };
    let opciones = PreparedChatCompletion {
        input: ChatInput::nuevo(vec![]), model_id: "tiny".into(), prompt_tokens: 5,
        max_new_tokens: 4, stream: false, include_usage: false, temperature: 0.0, top_p: 1.0, seed: Some(42),
    };
    let cargar = || load_cpu_backend(dir.to_str().unwrap(), perfil.clone()).unwrap();
    let mut primero = cargar();
    primero.habilitar_cache_kv(&tmp, b"prueba-binario-cpu", 4 * 1024 * 1024).unwrap();
    let prompt = [3, 4, 5, 6, 7];
    let (respuesta, _) = primero.generate_observed(&prompt, &opciones, &mut NoCancelObserver).unwrap();
    assert_eq!(soso_llm_api::kvstore_host::listar(&tmp).unwrap().len(), 1);
    let otro_prompt = [33, 34, 35, 36, 37];
    primero.generate_observed(&otro_prompt, &opciones, &mut NoCancelObserver).unwrap();
    assert_eq!(soso_llm_api::kvstore_host::listar(&tmp).unwrap().len(), 2);
    let mut continuacion = prompt.to_vec();
    continuacion.extend_from_slice(&respuesta);
    continuacion.extend_from_slice(&[8, 9]);
    drop(primero);
    let mut segundo = cargar();
    segundo.habilitar_cache_kv(&tmp, b"prueba-binario-cpu", 4 * 1024 * 1024).unwrap();
    let (caliente, _) = segundo.generate_observed(&continuacion, &opciones, &mut NoCancelObserver).unwrap();
    assert!(segundo.rt.ultimo_prefijo_reutilizado >= prompt.len());
    let mut frio = cargar();
    let (esperado, _) = frio.generate_observed(&continuacion, &opciones, &mut NoCancelObserver).unwrap();
    assert_eq!(caliente, esperado);
    for ruta in soso_llm_api::kvstore_host::listar(&tmp).unwrap() {
        std::fs::write(ruta, b"corrupta").unwrap();
    }
    let mut tercero = cargar();
    tercero.habilitar_cache_kv(&tmp, b"prueba-binario-cpu", 4 * 1024 * 1024).unwrap();
    let (recuperado, _) = tercero.generate_observed(&continuacion, &opciones, &mut NoCancelObserver).unwrap();
    assert_eq!(tercero.rt.ultimo_prefijo_reutilizado, 0);
    assert_eq!(recuperado, esperado);
    drop(tercero);
    let mut cuarto = cargar();
    cuarto.habilitar_cache_kv(&tmp, b"prueba-binario-cpu", 4 * 1024 * 1024).unwrap();
    let (restaurado, _) = cuarto.generate_observed(&continuacion, &opciones, &mut NoCancelObserver).unwrap();
    assert!(cuarto.rt.ultimo_prefijo_reutilizado > 0);
    assert_eq!(restaurado, esperado);
    std::fs::remove_dir_all(tmp).unwrap();
}

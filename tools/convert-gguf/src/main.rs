//! Convierte un fichero GGUF (llama o deepseek2 MLA) al layout sosomodel (.som).

use std::env;
use std::path::{Path, PathBuf};
use std::process;

use soso_llm_core::pipeline::PipelineRole;
use soso_llm_core::runtime::Runtime;
use sosomodel::{Manifest, TensorIndex};

fn check_model(dir: &Path) -> Result<(), String> {
    let manifest_path = dir.join("manifest.som");
    let index_path = dir.join("index.som");
    if !manifest_path.is_file() {
        return Err(format!("falta {}", manifest_path.display()));
    }
    if !index_path.is_file() {
        return Err(format!("falta {}", index_path.display()));
    }
    let manifest = Manifest::parse(&std::fs::read(&manifest_path).map_err(|e| e.to_string())?)
        .map_err(|_| String::from("manifest.som inválido"))?;
    let index = TensorIndex::parse(&std::fs::read(&index_path).map_err(|e| e.to_string())?)
        .map_err(|_| String::from("index.som inválido"))?;
    let rt = Runtime::new(manifest.clone(), index, 0, 0);
    rt.validate_shapes_for_role(PipelineRole::Full, 0, manifest.num_layers)
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--check") {
        args.remove(0);
        let dir = args.first().map(PathBuf::from).unwrap_or_else(|| {
            eprintln!("uso: convert-gguf --check <dir_modelo>");
            process::exit(2);
        });
        match check_model(&dir) {
            Ok(()) => {
                println!("convert-gguf: OK {}", dir.display());
            }
            Err(why) => {
                eprintln!("convert-gguf: {why}");
                process::exit(1);
            }
        }
        return;
    }

    let gguf = args.first().cloned().unwrap_or_else(|| {
        eprintln!("uso: convert-gguf <modelo.gguf> [dir_salida] [--name nombre]");
        eprintln!("     convert-gguf --check <dir_modelo>");
        process::exit(2);
    });
    let mut out = PathBuf::from("target/converted-model");
    let mut name = None;
    let mut pack_trunk = false;
    let mut rest: Vec<String> = args.into_iter().skip(1).collect();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--name" {
            name = rest.get(i + 1).cloned();
            rest.remove(i);
            rest.remove(i);
            continue;
        }
        if rest[i] == "--pack-trunk" {
            pack_trunk = true;
            rest.remove(i);
            continue;
        }
        if !rest[i].starts_with('-') && out == PathBuf::from("target/converted-model") {
            out = PathBuf::from(&rest[i]);
            rest.remove(i);
            continue;
        }
        i += 1;
    }
    if let Err(e) = gguf2som::convert_path_with_options(
        &gguf,
        &out,
        name.as_deref(),
        gguf2som::ConvertOptions::default().with_pack_trunk(pack_trunk),
    ) {
        eprintln!("convert-gguf: {e}");
        process::exit(1);
    }
    match check_model(&out) {
        Ok(()) => {
            println!("convert-gguf: shapes OK");
        }
        Err(why) => {
            eprintln!("convert-gguf: autocomprobación falló: {why}");
            process::exit(1);
        }
    }
    println!("convert-gguf: modelo escrito en {}", out.display());
}

//! Integra opcionalmente cubins previamente compilados; sin ellos se usa PTX.
use std::{collections::BTreeSet, env, fs, path::{Path, PathBuf}, process::Command};
use sha2::{Digest, Sha256};

fn hash(ruta: &Path) -> String {
    let datos = fs::read(ruta).unwrap_or_else(|e| panic!("{}: {e}", ruta.display()));
    Sha256::digest(datos).iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    println!("cargo:rerun-if-env-changed=SOSO_CUDA_AOT_DIR");
    println!("cargo:rerun-if-env-changed=SOSO_CUDA_AOT_SM");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let dir = env::var_os("SOSO_CUDA_AOT_DIR");
    let sm = env::var("SOSO_CUDA_AOT_SM").ok();
    let Some(dir) = dir else {
        assert!(sm.is_none(), "SOSO_CUDA_AOT_SM requiere SOSO_CUDA_AOT_DIR");
        fs::write(out.join("cuda_aot.rs"), "const AOT_SM: u32 = 0;\nconst AOT_Q4: Option<&[u8]> = None;\nconst AOT_IQ: Option<&[u8]> = None;\nconst AOT_CAPA: Option<&[u8]> = None;\n").unwrap();
        return;
    };
    let sm = sm.expect("SOSO_CUDA_AOT_DIR requiere SOSO_CUDA_AOT_SM");
    let arch: u32 = sm.strip_prefix("sm_").and_then(|x| x.parse().ok()).filter(|&n| (50..=255).contains(&n))
        .expect("SOSO_CUDA_AOT_SM debe ser sm_NN");
    let dir = PathBuf::from(dir);
    let manifiesto = dir.join("manifest.txt");
    println!("cargo:rerun-if-changed={}", manifiesto.display());
    let contenido = fs::read_to_string(&manifiesto).expect("falta manifest.txt; ejecutar scripts/build-cuda-aot.py");
    let mut lineas = contenido.lines();
    assert_eq!(lineas.next(), Some(format!("arch {sm}").as_str()), "arquitectura de manifiesto distinta");
    for nombre in ["matvec_q4_0.cu", "matvec_iq.cu", "gemma_capa.cu", "q4k_decode.h", "iq_tablas.cuh"] {
        let ruta = Path::new("../../lxdde/ports/nouveau").join(nombre);
        println!("cargo:rerun-if-changed={}", ruta.display());
        assert_eq!(lineas.next(), Some(format!("source {nombre} {}", hash(&ruta)).as_str()), "fuente AOT cambiada: {nombre}");
    }
    for nombre in ["matvec_q4_0", "matvec_iq", "gemma_capa"] {
        let cubin = dir.join(format!("{nombre}.cubin"));
        assert_eq!(lineas.next(), Some(format!("cubin {nombre}.cubin {}", hash(&cubin)).as_str()), "hash AOT inválido: {nombre}");
        println!("cargo:rerun-if-changed={}", cubin.display());
        let ptx = Path::new("cuda").join(format!("{nombre}.ptx"));
        println!("cargo:rerun-if-changed={}", ptx.display());
        validar(&cubin, &ptx, arch);
        fs::copy(cubin, out.join(format!("{nombre}.cubin"))).unwrap();
    }
    assert!(lineas.next().is_none(), "manifest AOT con líneas adicionales");
    fs::write(out.join("cuda_aot.rs"), format!(
        "const AOT_SM: u32 = {arch};\nconst AOT_Q4: Option<&[u8]> = Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/matvec_q4_0.cubin\")));\nconst AOT_IQ: Option<&[u8]> = Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/matvec_iq.cubin\")));\nconst AOT_CAPA: Option<&[u8]> = Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/gemma_capa.cubin\")));\n"
    )).unwrap();
}

fn validar(cubin: &Path, ptx: &Path, arch: u32) {
    let b = fs::read(cubin).unwrap_or_else(|e| panic!("{}: {e}", cubin.display()));
    assert!(b.len() >= 64 && &b[..4] == b"\x7fELF" && b[4] == 2 && b[5] == 1
        && u16::from_le_bytes([b[18], b[19]]) == 190 && b[48] as u32 == arch,
        "{} no es un cubin CUDA para sm_{arch}", cubin.display());
    let esperado: BTreeSet<_> = fs::read_to_string(ptx).unwrap().lines().filter_map(|linea| {
        let s = linea.trim().strip_prefix(".visible .entry ")?;
        Some(s.split('(').next()?.trim().to_owned())
    }).collect();
    assert!(!esperado.is_empty(), "PTX sin entradas: {}", ptx.display());
    let out = Command::new("readelf").arg("-sW").arg(cubin).output().expect("se requiere readelf para validar cubins");
    assert!(out.status.success(), "readelf falló para {}", cubin.display());
    let texto = String::from_utf8(out.stdout).expect("salida readelf inválida");
    let encontrados: BTreeSet<_> = texto.lines().filter(|l| l.contains(" FUNC ") && l.contains(" GLOBAL "))
        .filter_map(|l| l.split_whitespace().last().map(str::to_owned)).collect();
    let ausentes: Vec<_> = esperado.difference(&encontrados).collect();
    assert!(ausentes.is_empty(), "{} carece de kernels {:?}", cubin.display(), ausentes);
}

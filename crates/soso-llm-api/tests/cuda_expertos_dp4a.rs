//! Los kernels de experto de decode con producto entero (`dec_moe_gateup_s` / `dec_moe_down_s`, IQ2_S + IQ3_XXS/IQ4_XS) contra los de f32, que ya
//! están verificados frente a libggml. Con pesos aleatorios, sin modelo. Sin GPU la prueba se salta.
#![cfg(feature = "std")]

use soso_llm_api::cuda::CudaGpu;

fn probar(dd: i32, nombre: &str) {
    let mut gpu = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("sin CUDA ({e}): prueba omitida");
            return;
        }
    };
    // forma del 35B-A3B: ffn = 512, h = 2048, 256 expertos, top-8
    let r = gpu.probar_expertos_dp4a(dd, 256, 8, 512, 2048, 0x9E3779B97F4A7C15).expect("diagnóstico");
    eprintln!(
        "{nombre}: err gate/up {:.4}, err down {:.4}, err total {:.4} | f32 {:.1} us, dp4a {:.1} us",
        r.err_gateup, r.err_down, r.err_total, r.us_f32, r.us_dp4a
    );
    // la cuantización int8 de la activación (por bloques de 32) cuesta ~1 %; un error de indexado daría ~100 %
    assert!(r.err_gateup < 0.05, "{nombre}: gate/up {}", r.err_gateup);
    assert!(r.err_down < 0.05, "{nombre}: down {}", r.err_down);
    assert!(r.err_total < 0.08, "{nombre}: total {}", r.err_total);
}

#[test]
fn expertos_dp4a_iq3_xxs() {
    probar(18, "IQ2_S + IQ3_XXS");
}

#[test]
fn expertos_dp4a_iq4_xs() {
    probar(23, "IQ2_S + IQ4_XS");
}

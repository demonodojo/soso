//! Diagnóstico: geometría por capa de un modelo convertido.
use sosomodel::Manifest;
fn main() {
    let dir = std::env::args().nth(1).expect("dir");
    let m = Manifest::parse(&std::fs::read(format!("{dir}/manifest.som")).unwrap()).unwrap();
    println!("hidden {} capas {} max_seq {}", m.hidden_dim, m.num_layers, m.max_seq);
    for l in [0u32, 1, 5, 6, 11] {
        println!("capa {l}: heads {} kv {} hd {} ffn {} ventana {}", m.effective_num_heads(l), m.effective_num_kv_heads(l), m.effective_head_dim(l), m.effective_ffn_dim(l), m.layer_window(l));
    }
}

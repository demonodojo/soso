include!(concat!(env!("OUT_DIR"), "/gen.rs"));

pub const ETIQUETA: &str = env!("T41_ETIQUETA");

pub fn valor() -> i32 {
    GEN
}

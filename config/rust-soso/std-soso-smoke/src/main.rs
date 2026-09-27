//! Humo mínimo enlazado con la libstd cruzada de soso (T39 paso 4).
//!
//! `#![no_main]`: la PAL de std ya exporta `_start` (blob SOSA); un `fn main()`
//! normal haría que rustc generara otro `_start` y wild fallaría por símbolo duplicado.

#![no_main]

fn humo() {
    println!("std-soso-ok");
    for a in std::env::args().skip(1) {
        println!("arg={a}");
    }
}

#[no_mangle]
pub extern "C" fn main(_argc: isize, _argv: *const *const core::ffi::c_char) -> i32 {
    humo();
    0
}

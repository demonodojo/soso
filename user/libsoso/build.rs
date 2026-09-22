fn main() {
    println!("cargo::rustc-check-cfg=cfg(soso_heap_debug)");
    println!("cargo:rerun-if-env-changed=SOSO_HEAP_DEBUG");
    if heap_debug() {
        println!("cargo:rustc-cfg=soso_heap_debug");
    }
}

fn heap_debug() -> bool {
    matches!(
        std::env::var("SOSO_HEAP_DEBUG").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

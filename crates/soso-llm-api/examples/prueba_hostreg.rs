//! ¿Se puede anclar (`cuMemHostRegister`) un trozo del mapa del modelo, sin copiarlo? `prueba_hostreg <fichero> [MiB]`
use std::ffi::{c_char, c_int, c_void, CString};
unsafe extern "C" {
    fn dlopen(f: *const c_char, fl: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, s: *const c_char) -> *mut c_void;
    fn mmap(a: *mut c_void, l: usize, p: c_int, f: c_int, fd: c_int, o: i64) -> *mut c_void;
}
fn sym<T>(lib: *mut c_void, n: &str) -> T {
    let c = CString::new(n).unwrap();
    let p = unsafe { dlsym(lib, c.as_ptr()) };
    assert!(!p.is_null(), "{n}");
    unsafe { std::mem::transmute_copy(&p) }
}
fn main() {
    use std::os::fd::AsRawFd;
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mib: usize = a.get(1).map(|s| s.parse().unwrap()).unwrap_or(256);
    let f = std::fs::File::open(&a[0]).unwrap();
    let len = mib << 20;
    let p = unsafe { mmap(std::ptr::null_mut(), len, 1, 2, f.as_raw_fd(), 0) };
    assert!(p as isize != -1, "mmap");
    let lib = unsafe { dlopen(CString::new("libcuda.so.1").unwrap().as_ptr(), 2) };
    let init: unsafe extern "C" fn(c_int) -> c_int = sym(lib, "cuInit");
    let dev_get: unsafe extern "C" fn(*mut c_int, c_int) -> c_int = sym(lib, "cuDeviceGet");
    let retain: unsafe extern "C" fn(*mut *mut c_void, c_int) -> c_int = sym(lib, "cuDevicePrimaryCtxRetain");
    let setc: unsafe extern "C" fn(*mut c_void) -> c_int = sym(lib, "cuCtxSetCurrent");
    let reg: unsafe extern "C" fn(*mut c_void, usize, u32) -> c_int = sym(lib, "cuMemHostRegister_v2");
    let dev_ptr: unsafe extern "C" fn(*mut u64, *mut c_void, u32) -> c_int = sym(lib, "cuMemHostGetDevicePointer_v2");
    let alloc: unsafe extern "C" fn(*mut u64, usize) -> c_int = sym(lib, "cuMemAlloc_v2");
    let htod: unsafe extern "C" fn(u64, *const c_void, usize) -> c_int = sym(lib, "cuMemcpyHtoD_v2");
    let sync: unsafe extern "C" fn() -> c_int = sym(lib, "cuCtxSynchronize");
    unsafe {
        assert_eq!(init(0), 0);
        let (mut d, mut ctx) = (0, std::ptr::null_mut());
        assert_eq!(dev_get(&mut d, 0), 0);
        assert_eq!(retain(&mut ctx, d), 0);
        assert_eq!(setc(ctx), 0);
        let mut dp = 0u64;
        assert_eq!(alloc(&mut dp, len), 0);
        // referencia: copia desde memoria paginable
        let t = std::time::Instant::now();
        let r0 = htod(dp, p, len);
        sync();
        println!("htod paginable: rc {r0}, {:.1} ms ({:.1} GB/s)", t.elapsed().as_secs_f64() * 1e3, len as f64 / t.elapsed().as_secs_f64() / 1e9);
        for (nombre, fl) in [("READ_ONLY|PORTABLE", 0x09u32), ("READ_ONLY", 0x08), ("PORTABLE", 0x01), ("0", 0)] {
            let t = std::time::Instant::now();
            let rc = reg(p, len, fl);
            println!("cuMemHostRegister({nombre}) rc {rc} en {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
            if rc == 0 {
                let t = std::time::Instant::now();
                let r = htod(dp, p, len);
                sync();
                println!("  htod anclada: rc {r}, {:.1} ms ({:.1} GB/s)", t.elapsed().as_secs_f64() * 1e3, len as f64 / t.elapsed().as_secs_f64() / 1e9);
                let mut q = 0u64;
                println!("  devptr rc {}", dev_ptr(&mut q, p, 0));
                break;
            }
        }
    }
}

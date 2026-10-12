//! Tests de sosofs vía VFS del kernel (equivalente host de `crates/sosofs/tests/rename.rs`).

#![no_std]
#![no_main]

extern crate alloc;

use libsoso::{abi, println, sys};

libsoso::entry!(main);

fn check(cond: bool, msg: &str) -> u8 {
    if !cond {
        println!("FAIL: {msg}");
        1
    } else {
        0
    }
}

fn main(_args: &str) -> u8 {
    let mut fallo = 0u8;
    let _ = sys::mkdir("/tmp/sosofs-test");

    let fd = sys::open(
        "/tmp/sosofs-test/a",
        abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC,
    );
    fallo |= check(fd >= 0, "create a");
    if fd >= 0 {
        fallo |= check(
            sys::write_all(fd as u64, b"hola mundo").is_ok(),
            "write a",
        );
        sys::close(fd as u64);
    }

    fallo |= check(sys::rename("/tmp/sosofs-test/a", "/tmp/sosofs-test/b") == 0, "rename a→b");

    let fd = sys::open("/tmp/sosofs-test/b", abi::O_RDONLY);
    fallo |= check(fd >= 0, "open b");
    if fd >= 0 {
        let mut buf = [0u8; 16];
        let n = sys::read(fd as u64, &mut buf);
        fallo |= check(n == 10 && &buf[..10] == b"hola mundo", "read b");
        sys::close(fd as u64);
    }

    fallo |= check(sys::truncate("/tmp/sosofs-test/b", 4) == 0, "truncate 4");
    let fd = sys::open("/tmp/sosofs-test/b", abi::O_RDONLY);
    if fd >= 0 {
        let mut buf = [0u8; 8];
        let n = sys::read(fd as u64, &mut buf);
        fallo |= check(n == 4 && &buf[..4] == b"hola", "read truncated");
        sys::close(fd as u64);
    }

    let fd1 = sys::open(
        "/tmp/sosofs-test/c",
        abi::O_WRONLY | abi::O_CREAT | abi::O_EXCL,
    );
    fallo |= check(fd1 >= 0, "O_EXCL create");
    if fd1 >= 0 {
        sys::close(fd1 as u64);
    }
    let fd2 = sys::open(
        "/tmp/sosofs-test/c",
        abi::O_WRONLY | abi::O_CREAT | abi::O_EXCL,
    );
    fallo |= check(fd2 == -(abi::EEXIST as i64), "O_EXCL EEXIST");

    let _ = sys::unlink("/tmp/sosofs-test/b");
    let _ = sys::unlink("/tmp/sosofs-test/c");

    if fallo == 0 {
        println!("soso-test-sosofs: OK");
    }
    fallo
}

#![no_std]
#![no_main]

use core::arch::asm;

static MSG: [u8; 5] = *b"hola\n";
static mut CONT: u32 = 3;
static mut BUF: [u8; 64] = [0; 64];

#[panic_handler]
fn panico(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

unsafe extern "C" {
    fn sufijo() -> u32;
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        CONT += sufijo();
        BUF[0] = 1;
        asm!(
            "syscall",
            inlateout("rax") 2u64 => _,
            in("rdi") 1u64,
            in("rsi") MSG.as_ptr(),
            in("rdx") 5u64,
            lateout("rcx") _,
            lateout("r11") _,
        );
        asm!("syscall", in("rax") 0u64, in("rdi") (CONT + BUF[0] as u32) as u64, options(noreturn));
    }
}

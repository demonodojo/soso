//! Procesos de usuario: tabla, contextos y scheduler round-robin
//! preemptivo (solo se desaloja código de usuario; el kernel corre
//! siempre hasta completar).
//!
//! Diseño de pilas: el kernel no guarda estado entre entradas desde
//! usuario — todo vive en los `Process`. Cada entrada (syscall o
//! interrupción desde ring 3) empieza con la pila KSTACK vacía desde
//! arriba, y el scheduler la resetea al tomar el control, así que los
//! marcos abandonados no importan.

pub mod addrspace;
pub mod elf;
pub mod syscall;

use crate::arch::gdt;
use addrspace::{AddrSpace, STACK_SIZE, STACK_TOP};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::naked_asm;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;

/// Ticks de PIT (100 Hz) por rodaja de tiempo: 20 ms.
const TIMESLICE_TICKS: u32 = 2;
pub const MAX_FDS: usize = 16;

// ---- estado ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Runnable,
    /// Hasta uptime_ms >= t.
    Sleeping(u64),
    WaitingChild,
    /// read() de la tty sin datos: (puntero, longitud) del buffer usuario.
    WaitingTty { buf: u64, len: u64 },
    Zombie(u8),
}

/// Consola (tty) a la que está atada la E/S estándar de un proceso.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Console {
    /// Puerto serie: la consola física / de emergencia.
    Serial,
    /// Canal de una sesión SSH (sunset). Solo hay una sesión a la vez.
    Ssh,
}

impl Console {
    /// ¿Hay bytes disponibles para leer en esta consola?
    pub fn has_input(self) -> bool {
        match self {
            Console::Serial => crate::drivers::serial::has_input(),
            Console::Ssh => crate::net::ssh::rx_has_data(),
        }
    }

    /// Lee un byte sin bloquear.
    pub fn read_byte(self) -> Option<u8> {
        match self {
            Console::Serial => crate::drivers::serial::read_byte(),
            Console::Ssh => crate::net::ssh::rx_pop(),
        }
    }

    /// Escribe bytes crudos a la consola.
    pub fn write_bytes(self, data: &[u8]) {
        match self {
            Console::Serial => crate::drivers::serial::write_bytes(data),
            Console::Ssh => crate::net::ssh::tx_push(data),
        }
    }
}

/// Contexto de usuario para reanudar con iretq. El orden de los 15
/// registros coincide con los push del timer_isr (memoria ascendente).
#[repr(C)]
#[derive(Clone, Default)]
pub struct Context {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
}

const _: () = {
    assert!(core::mem::offset_of!(Context, rdi) == 64);
    assert!(core::mem::offset_of!(Context, rax) == 112);
    assert!(core::mem::offset_of!(Context, rip) == 120);
    assert!(core::mem::offset_of!(Context, rsp) == 128);
    assert!(core::mem::offset_of!(Context, rflags) == 136);
};

/// Descriptores de fichero. sosofs no tiene escritura parcial, así que
/// la lectura carga el fichero entero al abrir y la escritura acumula en
/// memoria y publica el fichero completo en close().
pub enum Fd {
    Tty,
    File { data: Vec<u8>, pos: usize },
    WriteBuf { dir: u64, name: String, data: Vec<u8>, pos: usize },
    Dir { entries: Vec<soso_abi::Dirent>, pos: usize },
}

pub struct Process {
    pub pid: u64,
    pub parent: u64,
    pub name: String,
    pub state: State,
    pub ctx: Context,
    /// None solo en zombies (el espacio se libera al morir).
    pub space: Option<AddrSpace>,
    pub fds: Vec<Option<Fd>>,
    pub brk: u64,
    pub brk_min: u64,
    /// Consola a la que van fd 0/1/2 (Fd::Tty).
    pub console: Console,
}

pub static PROCS: Mutex<Vec<Process>> = Mutex::new(Vec::new());
static CURRENT_PID: AtomicU64 = AtomicU64::new(0);
static NEXT_PID: AtomicU64 = AtomicU64::new(1);
static RR_NEXT: AtomicUsize = AtomicUsize::new(0);
static REMAINING: AtomicU32 = AtomicU32::new(0);

pub fn current_pid() -> u64 {
    CURRENT_PID.load(Ordering::Relaxed)
}

/// ¿Existe un proceso vivo (no zombie) con este pid?
pub fn exists(pid: u64) -> bool {
    PROCS.lock().iter().any(|p| p.pid == pid && !matches!(p.state, State::Zombie(_)))
}

/// Marca un proceso para morir (cliente SSH desconectado). No libera su
/// espacio aquí: si es el proceso actual no se puede; el scheduler recoge
/// los zombis huérfanos que no estén corriendo.
pub fn kill_pid(pid: u64) {
    let mut procs = PROCS.lock();
    if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
        p.state = State::Zombie(255);
        p.parent = 0;
    }
}

/// Ejecuta `f` con el proceso actual. Panica si no hay proceso actual.
pub fn with_current<R>(f: impl FnOnce(&mut Process) -> R) -> R {
    let pid = current_pid();
    let mut procs = PROCS.lock();
    let p = procs.iter_mut().find(|p| p.pid == pid).expect("sin proceso actual");
    f(p)
}

pub fn init() {
    syscall::init_msrs();
}

// ---- creación ----

pub fn spawn(path: &str, args: &str, parent: u64) -> Result<u64, i64> {
    spawn_console(path, args, parent, Console::Serial)
}

/// Como `spawn` pero atando la E/S estándar a la consola indicada
/// (p. ej. el canal SSH para la shell de una sesión remota).
pub fn spawn_console(
    path: &str,
    args: &str,
    parent: u64,
    console: Console,
) -> Result<u64, i64> {
    use soso_abi as abi;
    if args.len() > 3000 {
        return Err(-abi::EINVAL);
    }
    let data = {
        let fs = crate::fs::FS.get().ok_or(-abi::EIO)?;
        let mut fs = fs.lock();
        let ino = fs.resolve(path).map_err(syscall::fs_errno)?;
        fs.read_file(ino).map_err(syscall::fs_errno)?
    };
    let mut space = AddrSpace::new().ok_or(-abi::ENOMEM)?;
    match spawn_into(&mut space, &data, args) {
        Ok((ctx, brk)) => {
            let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
            PROCS.lock().push(Process {
                pid,
                parent,
                name: String::from(path),
                state: State::Runnable,
                ctx,
                space: Some(space),
                fds: vec![Some(Fd::Tty), Some(Fd::Tty), Some(Fd::Tty)],
                brk,
                brk_min: brk,
                console,
            });
            Ok(pid)
        }
        Err(e) => {
            space.free();
            Err(e)
        }
    }
}

fn spawn_into(space: &mut AddrSpace, data: &[u8], args: &str) -> Result<(Context, u64), i64> {
    use soso_abi as abi;
    let (entry, brk) = elf::load(space, data).map_err(|e| {
        crate::println!("spawn: elf inválido: {e}");
        -abi::EINVAL
    })?;
    for va in (STACK_TOP - STACK_SIZE..STACK_TOP).step_by(4096) {
        space.ensure_mapped(va).ok_or(-abi::ENOMEM)?;
    }
    // Los argumentos van en la última página de la pila; rsp queda debajo,
    // alineado a 16.
    let args_va = STACK_TOP - 4096;
    space.write(args_va, args.as_bytes()).ok_or(-abi::ENOMEM)?;
    let ctx = Context {
        rip: entry,
        rsp: args_va - 16,
        rflags: 0x202, // IF=1: sin ella no habría preempción
        rdi: args_va,
        rsi: args.len() as u64,
        ..Context::default()
    };
    Ok((ctx, brk))
}

// ---- salida y bloqueo ----

/// Empaqueta el retorno de wait(): (pid << 8) | código.
pub(crate) fn wait_pack(pid: u64, code: u8) -> u64 {
    (pid << 8) | code as u64
}

/// Termina el proceso actual y no vuelve.
pub fn exit_current(code: u8) -> ! {
    x86_64::instructions::interrupts::disable();
    addrspace::activate_kernel();
    let space;
    {
        let mut procs = PROCS.lock();
        let pid = current_pid();
        // Los hijos zombis mueren con él; los vivos pasan a huérfanos.
        procs.retain(|p| !(p.parent == pid && matches!(p.state, State::Zombie(_))));
        for p in procs.iter_mut() {
            if p.parent == pid {
                p.parent = 0;
            }
        }
        let idx = procs.iter().position(|p| p.pid == pid).expect("exit sin proceso");
        let parent = procs[idx].parent;
        let padre_esperando = procs
            .iter()
            .position(|p| p.pid == parent && matches!(p.state, State::WaitingChild));
        if let Some(pi) = padre_esperando {
            procs[pi].state = State::Runnable;
            procs[pi].ctx.rax = wait_pack(pid, code);
            space = procs.remove(idx).space;
        } else if parent != 0 && procs.iter().any(|p| p.pid == parent) {
            // El padre vive pero no espera: zombie hasta su wait().
            let p = &mut procs[idx];
            p.state = State::Zombie(code);
            space = p.space.take();
        } else {
            // Huérfano: nadie lo va a reclamar.
            space = procs.remove(idx).space;
        }
        CURRENT_PID.store(0, Ordering::Relaxed);
    }
    if let Some(s) = space {
        s.free();
    }
    schedule();
}

/// Mata el proceso actual por una falta (page fault, GP...).
pub fn kill_current(reason: &str) -> ! {
    let pid = current_pid();
    let name = with_current(|p| p.name.clone());
    crate::println!("task: [{pid}] {name} matado: {reason}");
    exit_current(255);
}

/// Bloquea el proceso actual con el contexto ya volcado en su `ctx`
/// (rax se rellenará al despertar) y cede el control.
pub fn block_current(ctx: Context, state: State) -> ! {
    x86_64::instructions::interrupts::disable();
    with_current(|p| {
        p.ctx = ctx;
        p.state = state;
    });
    CURRENT_PID.store(0, Ordering::Relaxed);
    schedule();
}

// ---- scheduler ----

/// Punto de entrada del scheduler: resetea la pila y no vuelve jamás.
pub fn schedule() -> ! {
    unsafe { schedule_landing() }
}

#[unsafe(naked)]
unsafe extern "C" fn schedule_landing() -> ! {
    naked_asm!(
        "cli",
        "lea rsp, [rip + {kstack}]",
        "add rsp, {size}",
        // KSTACK top está alineado a 16 (rsp%16==0). schedule_inner se
        // alcanza con `jmp`, no con `call`, así que emulamos el estado
        // post-call (rsp%16==8) restando 8; si no, los spills SSE de la
        // cripto (movaps) fallan con #GP por pila desalineada.
        "sub rsp, 8",
        "jmp {inner}",
        kstack = sym gdt::KSTACK,
        size = const gdt::KSTACK_SIZE,
        inner = sym schedule_inner,
    )
}

extern "C" fn schedule_inner() -> ! {
    loop {
        crate::net::poll();
        x86_64::instructions::interrupts::disable();
        let now = crate::arch::pit::uptime_ms();
        let mut procs = PROCS.lock();

        // Recoger zombis huérfanos (p. ej. shell de una sesión SSH que el
        // cliente cerró). Aquí no hay proceso corriendo (CURRENT=0), pero
        // el CR3 activo puede apuntar a un espacio que vamos a liberar.
        if procs.iter().any(|p| p.parent == 0 && matches!(p.state, State::Zombie(_))) {
            addrspace::activate_kernel();
            let mut i = 0;
            while i < procs.len() {
                if procs[i].parent == 0 && matches!(procs[i].state, State::Zombie(_)) {
                    if let Some(s) = procs.remove(i).space {
                        s.free();
                    }
                } else {
                    i += 1;
                }
            }
        }

        if procs.is_empty() {
            drop(procs);
            CURRENT_PID.store(0, Ordering::Relaxed);
            addrspace::activate_kernel();
            x86_64::instructions::interrupts::enable();
            crate::println!("task: no quedan procesos");
            crate::kshell::run();
        }

        // Despertares: temporizadores vencidos y lectores de tty con datos.
        for p in procs.iter_mut() {
            if let State::Sleeping(t) = p.state
                && now >= t
            {
                p.state = State::Runnable;
                p.ctx.rax = 0;
            }
        }
        // Despertar a cada lector de tty cuya consola ya tenga datos.
        for i in 0..procs.len() {
            if let State::WaitingTty { buf, len } = procs[i].state
                && procs[i].console.has_input()
            {
                // La copia necesita su espacio activo.
                procs[i].space.as_ref().unwrap().activate();
                let console = procs[i].console;
                procs[i].ctx.rax = tty_read_into(console, buf, len);
                procs[i].state = State::Runnable;
            }
        }

        // Round-robin desde el último elegido.
        let n = procs.len();
        let start = RR_NEXT.load(Ordering::Relaxed) % n;
        let pick =
            (0..n).map(|i| (start + i) % n).find(|&i| procs[i].state == State::Runnable);
        match pick {
            Some(i) => {
                RR_NEXT.store((i + 1) % n, Ordering::Relaxed);
                CURRENT_PID.store(procs[i].pid, Ordering::Relaxed);
                REMAINING.store(TIMESLICE_TICKS, Ordering::Relaxed);
                let ctx = procs[i].ctx.clone();
                procs[i].space.as_ref().expect("runnable sin espacio").activate();
                drop(procs);
                unsafe { resume_user(&ctx) }
            }
            None => {
                drop(procs);
                CURRENT_PID.store(0, Ordering::Relaxed);
                // Nada listo: dormir hasta la próxima interrupción.
                x86_64::instructions::interrupts::enable_and_hlt();
            }
        }
    }
}

/// Copia bytes de la consola al buffer de usuario (CR3 del proceso ya
/// activo; el rango se validó en la syscall). Devuelve cuántos.
fn tty_read_into(console: Console, buf: u64, len: u64) -> u64 {
    let mut n = 0u64;
    while n < len {
        match console.read_byte() {
            Some(b) => {
                unsafe { *((buf + n) as *mut u8) = b };
                n += 1;
            }
            None => break,
        }
    }
    n
}

/// Reanuda un contexto de usuario con iretq. Interrupciones deshabilitadas
/// al entrar; el rflags del contexto (IF=1) las rearma ya en ring 3.
#[unsafe(naked)]
unsafe extern "C" fn resume_user(ctx: &Context) -> ! {
    naked_asm!(
        // Marco iretq: ss, rsp, rflags, cs, rip (selectores fijados en gdt).
        "push 0x1b",
        "push [rdi + 128]", // rsp
        "push [rdi + 136]", // rflags
        "push 0x23",
        "push [rdi + 120]", // rip
        "mov r15, [rdi + 0]",
        "mov r14, [rdi + 8]",
        "mov r13, [rdi + 16]",
        "mov r12, [rdi + 24]",
        "mov r11, [rdi + 32]",
        "mov r10, [rdi + 40]",
        "mov r9,  [rdi + 48]",
        "mov r8,  [rdi + 56]",
        "mov rsi, [rdi + 72]",
        "mov rbp, [rdi + 80]",
        "mov rbx, [rdi + 88]",
        "mov rdx, [rdi + 96]",
        "mov rcx, [rdi + 104]",
        "mov rax, [rdi + 112]",
        "mov rdi, [rdi + 64]", // el puntero, al final
        "iretq",
    )
}

// ---- interrupción de timer (IRQ0) ----

/// Marco apilado por timer_isr: 15 registros (mismo orden que `Context`)
/// + el marco de interrupción de la CPU.
#[repr(C)]
struct TrapFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rbx: u64,
    rdx: u64,
    rcx: u64,
    rax: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

const _: () = assert!(core::mem::offset_of!(TrapFrame, rip) == 120);

#[unsafe(naked)]
pub extern "C" fn timer_isr() {
    naked_asm!(
        "push rax",
        "push rcx",
        "push rdx",
        "push rbx",
        "push rbp",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov rdi, rsp",
        // Marco de la CPU (rsp%16==8) + 15 pushes (120 B) => rsp%16==0
        // antes del call, que es justo lo que la ABI pide (el call empuja
        // 8 y timer_tick entra con rsp%16==8).
        "call {rust}",
        "test al, al",
        "jnz 2f",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "pop rbx",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "iretq",
        "2:",
        "jmp {sched}",
        rust = sym timer_tick,
        sched = sym schedule_landing,
    )
}

/// Devuelve 1 si hay que replanificar (el contexto ya quedó guardado).
extern "C" fn timer_tick(f: &mut TrapFrame) -> u64 {
    crate::arch::pit::tick();
    unsafe {
        crate::arch::interrupts::PICS
            .lock()
            .notify_end_of_interrupt(crate::arch::interrupts::InterruptIndex::Timer as u8);
    }
    // Solo se desaloja al usuario (ring 3); el kernel corre hasta acabar.
    if f.cs & 3 != 3 {
        return 0;
    }
    // Venimos de usuario: el kernel no sostiene ningún lock, se puede
    // atender la red aquí (si no, un proceso cpu-bound la mataría de hambre).
    crate::net::poll();
    let cur = CURRENT_PID.load(Ordering::Relaxed);
    if cur == 0 {
        return 0;
    }
    if REMAINING.fetch_sub(1, Ordering::Relaxed) > 1 {
        return 0;
    }
    let mut procs = PROCS.lock();
    let hay_otro =
        procs.iter().any(|p| p.pid != cur && p.state == State::Runnable);
    if !hay_otro {
        REMAINING.store(TIMESLICE_TICKS, Ordering::Relaxed);
        return 0;
    }
    if let Some(p) = procs.iter_mut().find(|p| p.pid == cur) {
        p.ctx = Context {
            r15: f.r15,
            r14: f.r14,
            r13: f.r13,
            r12: f.r12,
            r11: f.r11,
            r10: f.r10,
            r9: f.r9,
            r8: f.r8,
            rdi: f.rdi,
            rsi: f.rsi,
            rbp: f.rbp,
            rbx: f.rbx,
            rdx: f.rdx,
            rcx: f.rcx,
            rax: f.rax,
            rip: f.rip,
            rsp: f.rsp,
            rflags: f.rflags,
        };
        p.state = State::Runnable;
    }
    CURRENT_PID.store(0, Ordering::Relaxed);
    1
}

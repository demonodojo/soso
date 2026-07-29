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
pub mod futex;
pub mod mmap;
pub mod path;
pub mod pipe;
pub mod syscall;

use crate::arch::gdt;
use addrspace::{AddrSpace, STACK_SIZE, STACK_TOP};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::naked_asm;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator};

/// Ticks de PIT (100 Hz) por rodaja de tiempo: 20 ms.
const TIMESLICE_TICKS: u32 = 2;
pub const MAX_FDS: usize = 16;

// ---- estado ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Runnable,
    /// Elegido y en ejecución en algún core (a diferencia de `Runnable`, no
    /// se puede volver a elegir): con un solo core esto era implícito
    /// (nadie más miraba `PROCS` mientras corría), pero con SMP dos cores
    /// podrían adquirir el lock de `PROCS` en momentos distintos y ver el
    /// mismo proceso como `Runnable` mientras ya se ejecuta en otro core.
    Running,
    /// Hasta uptime_ms >= t.
    Sleeping(u64),
    WaitingChild,
    /// read() de la tty sin datos: (puntero, longitud) del buffer usuario.
    WaitingTty { buf: u64, len: u64 },
    /// read()/write() de un pipe sin datos o sin espacio.
    WaitingPipe {
        pipe_id: pipe::PipeId,
        buf: u64,
        len: u64,
        write: bool,
    },
    /// `futex_wait` sobre (pml4, uaddr).
    WaitingFutex { pml4: u64, uaddr: u64 },
    /// read/write/accept/connect TCP bloqueante.
    WaitingSocket {
        slot: usize,
        buf: u64,
        len: u64,
        write: bool,
        accept: bool,
        connect: bool,
        result_fd: u64,
        /// 0 = sin límite; si no, uptime_ms al que expira con -EAGAIN.
        deadline_ms: u64,
    },
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

/// Descriptores de fichero.
pub enum Fd {
    Tty,
    /// Fichero pequeño cargado entero al abrir.
    File { inode: u64, data: Vec<u8>, pos: usize },
    /// Fichero grande: lectura parcial bajo demanda.
    LazyFile { inode: u64, size: usize, pos: usize },
    WriteBuf { dir: u64, name: String, data: Vec<u8>, pos: usize },
    Dir { entries: Vec<soso_abi::Dirent>, pos: usize },
    PipeRead(pipe::PipeId),
    PipeWrite(pipe::PipeId),
    Tcp { slot: usize },
}

pub struct Process {
    pub pid: u64,
    pub parent: u64,
    pub name: String,
    pub state: State,
    pub ctx: Context,
    /// None solo en zombies (el espacio se libera al morir / al caer el
    /// último Arc si está compartido entre hilos).
    pub space: Option<AddrSpace>,
    pub fds: Vec<Option<Fd>>,
    pub brk: u64,
    pub brk_min: u64,
    /// Consola a la que van fd 0/1/2 (Fd::Tty).
    pub console: Console,
    /// Directorio de trabajo (ruta absoluta normalizada).
    pub cwd: String,
    /// Estado FPU/SSE (xsave) capturado al desalojar por timer; se
    /// restaura en cada reanudación.
    pub fpu: crate::arch::fpu::FpuArea,
    /// `kill_pid` mientras el proceso está `Running` en otro core: no se
    /// marca Zombie todavía (evitar UAF del AddrSpace); el timer del
    /// owner_cpu lo convierte en Zombie al desalojar.
    pub kill_pending: bool,
}

pub static PROCS: Mutex<Vec<Process>> = Mutex::new(Vec::new());
static NEXT_PID: AtomicU64 = AtomicU64::new(1);
/// Protegido por `PROCS` (solo se toca con el lock tomado): el reparto
/// round-robin es global, compartido entre todos los cores.
static RR_NEXT: AtomicUsize = AtomicUsize::new(0);
/// Cuenta atrás del timeslice, una por CPU: cada core desaloja al proceso
/// que él mismo está ejecutando, no el de otro.
static REMAINING: [AtomicU32; crate::arch::smp::MAX_CPUS] =
    [const { AtomicU32::new(0) }; crate::arch::smp::MAX_CPUS];

fn remaining() -> &'static AtomicU32 {
    &REMAINING[crate::arch::percpu::cpu_index()]
}

/// PID que ejecuta la CPU actual (0 = ninguno). Vía GS: cada core tiene su
/// propio valor, no hay un único "proceso actual" del sistema.
pub fn current_pid() -> u64 {
    crate::arch::percpu::current_pid()
}

/// ¿Existe un proceso vivo (no zombie) con este pid?
pub fn exists(pid: u64) -> bool {
    PROCS.lock().iter().any(|p| p.pid == pid && !matches!(p.state, State::Zombie(_)))
}

/// Marca un proceso para morir (cliente SSH desconectado). No libera su
/// espacio aquí. Si está `Running` en un core, solo pone `kill_pending`
/// para que ese core lo convierta en Zombie al desalojar (marcar Zombie
/// mientras sigue en ring 3 permitiría liberar el AddrSpace desde otro
/// core → UAF).
pub fn kill_pid(pid: u64) {
    let mut procs = PROCS.lock();
    if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
        p.parent = 0;
        if matches!(p.state, State::Zombie(_)) {
            return;
        }
        if p.state == State::Running {
            p.kill_pending = true;
        } else {
            p.state = State::Zombie(255);
            p.kill_pending = false;
        }
    }
}

/// Ejecuta `f` con el proceso actual. Panica si no hay proceso actual.
pub fn with_current<R>(f: impl FnOnce(&mut Process) -> R) -> R {
    let pid = current_pid();
    let mut procs = PROCS.lock();
    let p = procs.iter_mut().find(|p| p.pid == pid).expect("sin proceso actual");
    f(p)
}

/// Intenta resolver un page fault de usuario en una región mmap.
///
/// Si la región respalda un fichero y el fault cae en un tramo de 2 MiB
/// completo y alineado (VA y offset de fichero), se sirve con una página
/// grande: un bloque físico contiguo rellenado con una lectura directa del
/// FS (sin caché de bloques). Si no, página de 4 KiB.
pub fn handle_mmap_fault(addr: u64, is_write: bool) -> bool {
    const HUGE: u64 = 2 * 1024 * 1024;
    if current_pid() == 0 {
        return false;
    }
    with_current(|p| {
        let space = p.space.as_ref().unwrap().clone();
        let region = match space.find_mmap_region(addr) {
            Some(r) => r,
            None => return false,
        };
        if is_write && !region.writable {
            return false;
        }
        if space.is_mapped(addr & !0xfff) {
            return true;
        }
        // Presión de memoria: evictar pesos mmap RO antes de pedir frames nuevos.
        if !region.writable && region.inode != 0 {
            if !crate::mm::reclaim::ensure_free_frames(1) {
                return false;
            }
        }
        // Los guards de FRAME_ALLOC no pueden seguir vivos al llamar a
        // map_page*, que toma el mismo spinlock para los frames de tablas.
        let free_frame = |frame| unsafe {
            crate::mm::FRAME_ALLOC.get().unwrap().lock().deallocate_frame(frame);
        };

        // --- camino de página grande (2 MiB) ---
        let va_2m = addr & !(HUGE - 1);
        let off_2m = region.file_offset + va_2m.saturating_sub(region.virt_start);
        let huge_ok = region.inode != 0
            && va_2m >= region.virt_start
            && va_2m + HUGE <= region.virt_start + region.len
            && off_2m % HUGE == 0
            && off_2m + HUGE <= region.file_len;
        if huge_ok {
            if !region.writable && region.inode != 0 {
                let _ = crate::mm::reclaim::ensure_free_frames(512);
            }
            let frame2m = crate::mm::FRAME_ALLOC.get().unwrap().lock().allocate_2m();
            if let Some(frame) = frame2m {
                let dst = unsafe {
                    core::slice::from_raw_parts_mut(
                        crate::mm::phys_to_virt(frame.start_address().as_u64()).as_mut_ptr::<u8>(),
                        HUGE as usize,
                    )
                };
                if crate::fs::load_file_range(region.inode, off_2m as usize, dst).is_ok()
                    && space.map_page_2m(va_2m, frame, region.writable).is_some()
                {
                    if !region.writable && region.inode != 0 {
                        crate::mm::reclaim::register(&space, va_2m, true);
                    }
                    return true;
                }
                unsafe {
                    crate::mm::FRAME_ALLOC.get().unwrap().lock().deallocate_2m(frame);
                }
                return false;
            }
            // sin bloque contiguo libre: se sirve con páginas de 4 KiB
        }

        // --- camino de página de 4 KiB ---
        let page_va = addr & !0xfff;
        let page_off = page_va - region.virt_start;
        let file_off = (region.file_offset + page_off) as usize;
        let frame = {
            let mut fa = crate::mm::FRAME_ALLOC.get().unwrap().lock();
            match fa.allocate_frame() {
                Some(f) => f,
                None => return false,
            }
        };
        // rellenar el frame directamente (sin buffer de 4 KiB en la pila);
        // la lectura se recorta al tamaño del fichero (última página
        // parcial → resto a cero)
        let dst = unsafe {
            &mut *crate::mm::phys_to_virt(frame.start_address().as_u64())
                .as_mut_ptr::<[u8; 4096]>()
        };
        dst.fill(0);
        if region.inode != 0 {
            let avail = (region.file_len as usize).saturating_sub(file_off).min(4096);
            if avail == 0
                || crate::vfs::read_file_range(region.inode, file_off, avail, &mut dst[..avail])
                    .is_err()
            {
                free_frame(frame);
                return false;
            }
        }
        if space.map_page(page_va, frame, region.writable).is_none() {
            free_frame(frame);
            return false;
        }
        if !region.writable && region.inode != 0 {
            crate::mm::reclaim::register(&space, page_va, false);
        }
        true
    })
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
    spawn_console_io(path, args, parent, console, [soso_abi::FD_INHERIT_TTY; 3])
}

/// Como `spawn_console` pero con stdio opcional (u64::MAX = tty).
pub fn spawn_console_io(
    path: &str,
    args: &str,
    parent: u64,
    console: Console,
    stdio: [u64; 3],
) -> Result<u64, i64> {
    use soso_abi as abi;
    if args.len() > 3000 {
        return Err(-abi::EINVAL);
    }
    let stdio_fds = if stdio.iter().all(|&f| f == abi::FD_INHERIT_TTY) {
        [None, None, None]
    } else if current_pid() == 0 {
        return Err(-abi::EINVAL);
    } else {
        syscall::take_stdio_fds(stdio)?
    };
    let cwd = {
        let procs = PROCS.lock();
        procs
            .iter()
            .find(|p| p.pid == parent)
            .map(|p| p.cwd.clone())
            .unwrap_or_else(|| String::from("/"))
    };
    let data = {
        let ino = crate::vfs::resolve(path).map_err(crate::task::syscall::fs_errno)?;
        crate::vfs::read_file(ino).map_err(crate::task::syscall::fs_errno)?
    };
    let space = AddrSpace::new().ok_or(-abi::ENOMEM)?;
    match spawn_into(&space, &data, args) {
        Ok((ctx, brk)) => {
            let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
            let mut fds = vec![Some(Fd::Tty), Some(Fd::Tty), Some(Fd::Tty)];
            for (slot, fd) in stdio_fds.into_iter().enumerate() {
                if let Some(f) = fd {
                    fds[slot] = Some(f);
                }
            }
            PROCS.lock().push(Process {
                pid,
                parent,
                name: String::from(path),
                state: State::Runnable,
                ctx,
                space: Some(space),
                fds,
                brk,
                brk_min: brk,
                console,
                cwd,
                fpu: crate::arch::fpu::FpuArea::inicial(),
                kill_pending: false,
            });
            crate::arch::apic::kick_idle_cpus();
            Ok(pid)
        }
        Err(e) => {
            drop(space);
            Err(e)
        }
    }
}

/// Crea un hilo: mismo AddrSpace (Arc), pila y entry proporcionados por
/// el usuario. Devuelve el tid (= pid).
pub fn thread_spawn(entry: u64, arg: u64, stack_top: u64) -> Result<u64, i64> {
    use soso_abi as abi;
    if entry == 0 || stack_top == 0 || stack_top > addrspace::USER_MAX {
        return Err(-abi::EINVAL);
    }
    let parent = current_pid();
    if parent == 0 {
        return Err(-abi::EINVAL);
    }
    let (space, console, cwd, brk, brk_min, name) = with_current(|p| {
        let space = p.space.as_ref().ok_or(-abi::ENOMEM)?.clone();
        Ok::<_, i64>((
            space,
            p.console,
            p.cwd.clone(),
            p.brk,
            p.brk_min,
            p.name.clone(),
        ))
    })?;
    let tid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
    // ABI SysV: en la entrada de una función (como tras un `call`)
    // `rsp % 16 == 8`. El iretq no hace `call`, así que dejamos la pila
    // 8 bytes por debajo del alineado a 16.
    let rsp = (stack_top & !0xFu64).wrapping_sub(8);
    let ctx = Context {
        rip: entry,
        rsp,
        rflags: 0x202,
        rdi: arg,
        ..Context::default()
    };
    PROCS.lock().push(Process {
        pid: tid,
        parent,
        name,
        state: State::Runnable,
        ctx,
        space: Some(space),
        fds: vec![Some(Fd::Tty), Some(Fd::Tty), Some(Fd::Tty)],
        brk,
        brk_min,
        console,
        cwd,
        fpu: crate::arch::fpu::FpuArea::inicial(),
        kill_pending: false,
    });
    crate::arch::apic::kick_idle_cpus();
    Ok(tid)
}

fn spawn_into(space: &AddrSpace, data: &[u8], args: &str) -> Result<(Context, u64), i64> {
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
        futex::forget_pid(pid);
        syscall::close_all_fds(&mut procs[idx].fds);
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
        crate::arch::percpu::set_current_pid(0);
    }
    drop(space);
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
    crate::arch::percpu::set_current_pid(0);
    schedule();
}

// ---- scheduler ----

/// Punto de entrada del scheduler: resetea la pila (la de ESTE core) y no
/// vuelve jamás. `block_current`/`exit_current`/etc. llaman aquí desde
/// cualquier core (p. ej. un proceso bloqueándose en un AP) — despachar al
/// landing equivocado saltaría a la pila de la BSP mientras esta puede
/// estar en uso a la vez: corrupción garantizada, no solo posible. Este era
/// el bug real detrás de las caídas intermitentes al activar el scheduler
/// multicore: todo lo demás (percpu, TSS, timer/syscall de los APs) estaba
/// bien, pero `schedule()` seguía yendo siempre al landing de la BSP.
pub fn schedule() -> ! {
    if crate::arch::percpu::cpu_index() == 0 {
        unsafe { schedule_landing() }
    } else {
        unsafe { ap_schedule_landing() }
    }
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

/// Punto de entrada del scheduler para un AP: como `schedule_landing` pero
/// con la pila de ESTE core (vía GS, no un símbolo fijo — cada AP tiene la
/// suya, `gdt::kstack_top_for`/`arch::percpu`). Reutiliza `schedule_inner`
/// tal cual: el reparto de procesos ya es multicore-seguro (lock de
/// `PROCS`, `State::Running`, `CURRENT_PID`/timeslice per-CPU).
#[unsafe(naked)]
unsafe extern "C" fn ap_schedule_landing() -> ! {
    naked_asm!(
        "cli",
        "mov rsp, gs:[{kstack}]",
        "sub rsp, 8",
        "jmp {inner}",
        kstack = const crate::arch::percpu::OFF_KSTACK_TOP,
        inner = sym schedule_inner,
    )
}

/// Llamada una vez por un AP tras terminar su arranque: entra en el
/// scheduler multicore y no vuelve jamás.
pub fn ap_enter_scheduler() -> ! {
    unsafe { ap_schedule_landing() }
}

extern "C" fn schedule_inner() -> ! {
    loop {
        // Solo la BSP atiende la red y cae al kernel-shell si no quedan
        // procesos: es la consola local, y net::poll ya usa try_lock (un
        // AP que también llamara aquí solo desperdiciaría ciclos).
        let es_bsp = crate::arch::percpu::cpu_index() == 0;
        if es_bsp {
            crate::net::poll();
            #[cfg(feature = "lxdde")]
            crate::lxdde::poll();
        }
        x86_64::instructions::interrupts::disable();
        let now = crate::arch::pit::uptime_ms();
        let mut procs = PROCS.lock();

        // Recoger zombis huérfanos (p. ej. shell de una sesión SSH que el
        // cliente cerró). No liberar el AddrSpace si el pid sigue en
        // ejecución en algún core (race kill_pid → UAF).
        if procs.iter().any(|p| p.parent == 0 && matches!(p.state, State::Zombie(_))) {
            addrspace::activate_kernel();
            let mut i = 0;
            while i < procs.len() {
                if procs[i].parent == 0
                    && matches!(procs[i].state, State::Zombie(_))
                    && !crate::arch::percpu::pid_en_ejecucion(procs[i].pid)
                {
                    drop(procs.remove(i).space);
                } else {
                    i += 1;
                }
            }
        }

        if procs.is_empty() {
            drop(procs);
            crate::arch::percpu::set_current_pid(0);
            addrspace::activate_kernel();
            if es_bsp {
                x86_64::instructions::interrupts::enable();
                crate::println!("task: no quedan procesos");
                crate::kshell::run();
            }
            // Un AP sin procesos que repartir: igual que "nada listo todavía".
            x86_64::instructions::interrupts::enable_and_hlt();
            continue;
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
                // Y revalidar el rango, igual que en pipe y socket: se comprobó al
                // entrar en `sys_read`, pero otro hilo pudo desmapearlo durante la
                // espera y `tty_read_into` escribe con un puntero pelado.
                if !procs[i].space.as_ref().unwrap().range_ok(buf, len, true) {
                    procs[i].ctx.rax = (-soso_abi::EFAULT) as u64;
                    procs[i].state = State::Runnable;
                    continue;
                }
                let console = procs[i].console;
                procs[i].ctx.rax = tty_read_into(console, buf, len);
                procs[i].state = State::Runnable;
            }
        }
        // Despertar lectores/escritores de pipe cuando haya datos, espacio o EOF.
        for i in 0..procs.len() {
            if let State::WaitingPipe { pipe_id, buf, len, write } = procs[i].state {
                procs[i].space.as_ref().unwrap().activate();
                // Revalidar el rango del usuario: se comprobó al entrar en la
                // syscall, pero otro hilo del proceso pudo hacer `munmap` mientras
                // este estaba bloqueado, y aquí el kernel copia sin red. Se usa
                // `AddrSpace::range_ok`, que no toma candados (ya tenemos PROCS).
                if !procs[i]
                    .space
                    .as_ref()
                    .unwrap()
                    .range_ok(buf, len, !write)
                {
                    procs[i].ctx.rax = (-soso_abi::EFAULT) as u64;
                    procs[i].state = State::Runnable;
                    continue;
                }
                if write {
                    match pipe_wake_write(pipe_id, buf, len) {
                        Ok(n) if n > 0 => {
                            procs[i].ctx.rax = n;
                            procs[i].state = State::Runnable;
                        }
                        Err(e) => {
                            procs[i].ctx.rax = e as u64;
                            procs[i].state = State::Runnable;
                        }
                        _ => {}
                    }
                } else {
                    let n = pipe_wake_read(pipe_id, buf, len);
                    if n > 0 || pipe::write_closed(pipe_id) {
                        procs[i].ctx.rax = n;
                        procs[i].state = State::Runnable;
                    }
                }
            }
        }
        // Despertar operaciones TCP bloqueantes.
        for i in 0..procs.len() {
            if let State::WaitingSocket {
                slot,
                buf,
                len,
                write,
                accept,
                connect,
                result_fd,
                deadline_ms,
            } = procs[i].state
            {
                procs[i].space.as_ref().unwrap().activate();
                let now = crate::arch::pit::uptime_ms();
                if deadline_ms != 0 && now >= deadline_ms {
                    procs[i].ctx.rax = (-soso_abi::EAGAIN) as u64;
                    procs[i].state = State::Runnable;
                    continue;
                }
                if accept {
                    if crate::net::tcp_listener_ready(slot) {
                        let _ = crate::net::tcp_accept(slot);
                        procs[i].ctx.rax = result_fd;
                        procs[i].state = State::Runnable;
                    }
                } else if connect {
                    if crate::net::tcp_is_connected(slot) {
                        procs[i].ctx.rax = result_fd;
                        procs[i].state = State::Runnable;
                    } else if crate::net::tcp_connect_failed(slot) {
                        procs[i].ctx.rax = (-soso_abi::ECONNREFUSED) as u64;
                        procs[i].state = State::Runnable;
                    }
                } else {
                    // Igual que en el pipe: el rango se validó al entrar en la
                    // syscall, pero otro hilo pudo hacer `munmap` durante el
                    // bloqueo, y estas dos ramas copian del/al búfer del usuario.
                    // `accept`/`connect` no tocan memoria y quedan fuera.
                    if !procs[i].space.as_ref().unwrap().range_ok(buf, len, !write) {
                        procs[i].ctx.rax = (-soso_abi::EFAULT) as u64;
                        procs[i].state = State::Runnable;
                        continue;
                    }
                    if write {
                        match crate::net::tcp_try_write(slot, buf, len) {
                            Ok(n) if n > 0 => {
                                procs[i].ctx.rax = n;
                                procs[i].state = State::Runnable;
                            }
                            _ => {}
                        }
                    } else {
                        match crate::net::tcp_try_read(slot, buf, len) {
                            Ok(n) if n > 0 => {
                                procs[i].ctx.rax = n;
                                procs[i].state = State::Runnable;
                            }
                            Ok(0) if !crate::net::tcp_is_connected(slot) => {
                                procs[i].ctx.rax = 0;
                                procs[i].state = State::Runnable;
                            }
                            _ => {}
                        }
                    }
                }
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
                // Marca "en ejecución": ningún otro core puede volver a
                // elegir este pid mientras esté así (ver doc de `Running`).
                procs[i].state = State::Running;
                crate::arch::percpu::set_current_pid(procs[i].pid);
                remaining().store(TIMESLICE_TICKS, Ordering::Relaxed);
                let ctx = procs[i].ctx.clone();
                procs[i].space.as_ref().expect("runnable sin espacio").activate();
                // Restaurar el estado FPU del proceso justo antes de saltar
                // a usuario (después de esto, nada de SSE en este camino).
                unsafe { crate::arch::fpu::restore(&procs[i].fpu) };
                drop(procs);
                unsafe { resume_user(&ctx) }
            }
            None => {
                drop(procs);
                crate::arch::percpu::set_current_pid(0);
                // Nada listo: dormir hasta la próxima interrupción.
                x86_64::instructions::interrupts::enable_and_hlt();
            }
        }
    }
}

fn pipe_wake_read(pipe_id: pipe::PipeId, buf: u64, len: u64) -> u64 {
    if pipe::has_data(pipe_id) || pipe::write_closed(pipe_id) {
        pipe::try_read(pipe_id, buf, len)
    } else {
        0
    }
}

fn pipe_wake_write(pipe_id: pipe::PipeId, buf: u64, len: u64) -> Result<u64, i64> {
    if pipe::no_readers(pipe_id) {
        return Err(-soso_abi::EPIPE);
    }
    if pipe::has_space(pipe_id) {
        pipe::try_write(pipe_id, buf, len)
    } else {
        Ok(0)
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
        // Preservar el estado FPU/SSE/AVX del contexto interrumpido ANTES
        // de net::poll (la cripto clobbea XMM). Si timer_tick desaloja,
        // copia TIMER_FPU al Process; si no, se restaura aquí al salir.
        // (rax/rdx ya están salvados en los push de arriba.)
        "mov eax, 7",
        "xor edx, edx",
        "xsave64 [rip + {fpu}]",
        // rdi = marco para timer_tick. rsp%16 tras los 15 pushes depende
        // del alineamiento del usuario; net::poll (cripto) exige rsp%16==8
        // al entrar en timer_tick (rsp%16==0 justo antes del call).
        "mov rdi, rsp",
        "test rsp, 8",
        "jz 1f",
        "sub rsp, 8",
        "call {rust}",
        "add rsp, 8",
        "jmp 3f",
        "1:",
        "call {rust}",
        "3:",
        "test al, al",
        "jnz 2f",
        "mov eax, 7",
        "xor edx, edx",
        "xrstor64 [rip + {fpu}]",
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
        fpu = sym crate::arch::fpu::TIMER_FPU,
    )
}

/// Timer LAPIC de un AP: mismo esquema que `timer_isr`, pero el área xsave
/// y la pila de reentrada al scheduler son las de ESTE core (vía GS, no un
/// símbolo fijo) — dos APs preemptando a la vez no pueden pisarse.
#[unsafe(naked)]
pub extern "C" fn ap_timer_isr() {
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
        // r11 libre (ya salvado arriba): puntero al FpuArea de este core.
        "mov r11, gs:[{fpu_off}]",
        "mov eax, 7",
        "xor edx, edx",
        "xsave64 [r11]",
        "mov rdi, rsp",
        "test rsp, 8",
        "jz 1f",
        "sub rsp, 8",
        "call {rust}",
        "add rsp, 8",
        "jmp 3f",
        "1:",
        "call {rust}",
        "3:",
        "test al, al",
        "jnz 2f",
        "mov r11, gs:[{fpu_off}]",
        "mov eax, 7",
        "xor edx, edx",
        "xrstor64 [r11]",
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
        rust = sym ap_timer_tick,
        sched = sym ap_schedule_landing,
        fpu_off = const crate::arch::percpu::OFF_FPU_SCRATCH,
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
    #[cfg(feature = "lxdde")]
    crate::lxdde::poll();
    let cur = crate::arch::percpu::current_pid();
    if cur == 0 {
        return 0;
    }
    desalojar_si_toca(f, cur, /*bsp_fpu*/ true)
}

/// Igual que `timer_tick` pero para el timer LAPIC de un AP: EOI del LAPIC
/// en vez del PIC, sin `pit::tick()` (el reloj global lo lleva solo la BSP,
/// que es la única con el PIT/PIC) y sin `net::poll()` (solo la BSP la
/// atiende, ver `schedule_inner`).
extern "C" fn ap_timer_tick(f: &mut TrapFrame) -> u64 {
    crate::arch::apic::eoi();
    if f.cs & 3 != 3 {
        return 0;
    }
    let cur = crate::arch::percpu::current_pid();
    if cur == 0 {
        return 0;
    }
    desalojar_si_toca(f, cur, /*bsp_fpu*/ false)
}

/// Lógica común de preempción: respeta `kill_pending`/`Zombie` (nunca
/// Zombie→Runnable) y fuerza el desalojo si hay kill aunque no haya otro
/// Runnable.
fn desalojar_si_toca(f: &mut TrapFrame, cur: u64, bsp_fpu: bool) -> u64 {
    let mut procs = PROCS.lock();
    let force_kill = procs
        .iter()
        .any(|p| p.pid == cur && (p.kill_pending || matches!(p.state, State::Zombie(_))));
    if !force_kill && remaining().fetch_sub(1, Ordering::Relaxed) > 1 {
        return 0;
    }
    let hay_otro = procs
        .iter()
        .any(|p| p.pid != cur && p.state == State::Runnable);
    if !force_kill && !hay_otro {
        remaining().store(TIMESLICE_TICKS, Ordering::Relaxed);
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
        if bsp_fpu {
            p.fpu = unsafe { (*(&raw const crate::arch::fpu::TIMER_FPU)).clone() };
        } else {
            p.fpu = unsafe {
                (*(crate::arch::percpu::fpu_scratch_ptr() as *const crate::arch::fpu::FpuArea))
                    .clone()
            };
        }
        if p.kill_pending || matches!(p.state, State::Zombie(_)) {
            p.kill_pending = false;
            p.parent = 0;
            p.state = State::Zombie(255);
        } else {
            // Solo Running → Runnable; no tocar Waiting*/Sleeping.
            p.state = State::Runnable;
        }
    }
    crate::arch::percpu::set_current_pid(0);
    1
}

//! Manejadores de señales: disposición, máscara, pendientes y entrega.
//!
//! **Un solo punto de inyección.** Un manejador se arma siempre en el
//! planificador, justo antes de reanudar al proceso (`entregar_pendientes` en
//! `schedule_inner`): ahí `Process.ctx` y `Process.fpu` tienen el estado
//! completo de ring 3, venga de donde venga el proceso —despertado de un
//! bloqueo con `EINTR`, desalojado por el temporizador o saliendo de una
//! syscall que no bloquea—. Los otros dos caminos sólo se encargan de llevarlo
//! hasta allí: `desalojar_si_toca` fuerza el desalojo si hay una señal
//! entregable, y `al_volver_de_syscall` lo aparca como `Runnable` en vez de
//! volver con `sysret`. Así no hace falta ensamblador nuevo.
//!
//! **`sigreturn` lee datos del usuario y los carga en la CPU.** Todo lo que
//! venga del marco se valida antes: un `rip`/`rsp` no canónico haría saltar
//! `iretq` con #GP en ring 0, y una cabecera XSAVE o un MXCSR con bits
//! reservados harían lo mismo dentro de `xrstor`. Es un fallo del proceso,
//! nunca del kernel.

use super::addrspace::{AddrSpace, STACK_MAX, STACK_TOP, USER_MAX};
use super::{Context, Process, State};
use core::sync::atomic::{AtomicBool, Ordering};
use soso_abi as abi;

/// Estado de señales de un hilo. Las acciones son del proceso (se copian a
/// todos los hilos del mismo espacio en cada `sigaction`); la máscara y las
/// pendientes son de cada hilo.
#[derive(Clone)]
pub struct Estado {
    pub acciones: [u64; abi::NSIG],
    /// A dónde vuelve el manejador con `ret` (la rutina que llama a
    /// `SYS_SIGRETURN`).
    pub restaurador: u64,
    pub mascara: u64,
    pub pendientes: u64,
    /// Parado por SIGSTOP/SIGTSTP: el planificador no lo elige hasta SIGCONT.
    pub detenido: bool,
    /// `uptime_ms` en que vence la alarma (`SYS_ALARM`); 0 = sin alarma.
    pub alarma_ms: u64,
    /// Parada o continuación que el padre todavía no ha visto con `waitpid`.
    pub aviso_wait: Option<(u8, u8)>,
}

impl Estado {
    /// Estado de un proceso recién lanzado: todo por defecto, nada bloqueado.
    ///
    /// No se hereda nada del padre, tampoco lo ignorado: sosh ignora SIGTSTP
    /// y SIGQUIT, y si sus hijos lo heredaran Ctrl-Z no pararía a ninguno.
    pub const fn nuevo() -> Self {
        Self {
            acciones: [abi::SIG_DFL; abi::NSIG],
            restaurador: 0,
            mascara: 0,
            pendientes: 0,
            detenido: false,
            alarma_ms: 0,
            aviso_wait: None,
        }
    }

    /// Un hilo nuevo: las acciones del proceso y la máscara del hilo que lo
    /// crea; nada pendiente.
    pub fn para_hilo(&self) -> Self {
        Self {
            acciones: self.acciones,
            restaurador: self.restaurador,
            mascara: self.mascara,
            pendientes: 0,
            detenido: self.detenido,
            alarma_ms: 0,
            aviso_wait: None,
        }
    }

    /// Pendientes que no están bloqueadas.
    pub fn entregables(&self) -> u64 {
        self.pendientes & !self.mascara
    }
}

/// Qué pasa con una señal cuya disposición es [`abi::SIG_DFL`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Defecto {
    Terminar,
    Ignorar,
    Parar,
    Continuar,
}

pub fn defecto(sig: u64) -> Defecto {
    match sig {
        abi::SIGCHLD | abi::SIGWINCH => Defecto::Ignorar,
        abi::SIGCONT => Defecto::Continuar,
        abi::SIGSTOP | abi::SIGTSTP => Defecto::Parar,
        _ => Defecto::Terminar,
    }
}

/// Puede que algún proceso **en ejecución** tenga una señal entregable o esté
/// parado. Lo mira la salida de cada syscall antes de tomar `PROCS`: si está a
/// `false` no hace falta. Lo pone quien genera la señal y lo recalcula el
/// planificador; si alguien lo lee a destiempo, el temporizador siguiente lo
/// arregla (`desalojar_si_toca` mira el proceso con el candado cogido).
pub static QUIZA: AtomicBool = AtomicBool::new(false);

pub fn marcar() {
    QUIZA.store(true, Ordering::Relaxed);
}

/// Lo que el planificador tiene que hacer con un proceso tras mirar sus
/// pendientes.
pub enum Entrega {
    Nada,
    Inyectada,
    /// Disposición por defecto «terminar» (o pila inservible): código de salida.
    Terminar(u8),
    /// Disposición por defecto «parar»: la señal que lo para.
    Parar(u8),
}

/// Saca la primera señal entregable y la resuelve. Con manejador, deja el
/// marco en la pila del usuario y apunta `ctx` al manejador.
pub fn preparar(p: &mut Process) -> Entrega {
    loop {
        let listas = p.senales.entregables();
        if listas == 0 {
            return Entrega::Nada;
        }
        let sig = listas.trailing_zeros() as u64;
        p.senales.pendientes &= !abi::sig_bit(sig);
        match p.senales.acciones[sig as usize] {
            abi::SIG_IGN => continue,
            abi::SIG_DFL => match defecto(sig) {
                Defecto::Ignorar | Defecto::Continuar => continue,
                Defecto::Terminar => return Entrega::Terminar(abi::exit_by_signal(sig as u8)),
                Defecto::Parar => return Entrega::Parar(sig as u8),
            },
            manejador => return inyectar(p, sig, manejador),
        }
    }
}

fn ctx_a_regs(ctx: &Context) -> [u64; 18] {
    // `Context` es `repr(C)` con 18 `u64` y nada más (comprobado abajo).
    unsafe { core::mem::transmute::<Context, [u64; 18]>(ctx.clone()) }
}

fn regs_a_ctx(regs: [u64; 18]) -> Context {
    unsafe { core::mem::transmute::<[u64; 18], Context>(regs) }
}

const _: () = assert!(core::mem::size_of::<Context>() == 18 * 8);

fn como_bytes(marco: &abi::SigFrame) -> &[u8] {
    unsafe {
        core::slice::from_raw_parts(
            (marco as *const abi::SigFrame).cast::<u8>(),
            core::mem::size_of::<abi::SigFrame>(),
        )
    }
}

fn inyectar(p: &mut Process, sig: u64, manejador: u64) -> Entrega {
    let Some(space) = p.space.as_ref() else {
        return Entrega::Nada;
    };
    let marco = abi::SigFrame {
        restaurador: p.senales.restaurador,
        sig,
        mascara: p.senales.mascara,
        _reservado: 0,
        regs: ctx_a_regs(&p.ctx),
        fpu: p.fpu.0,
    };
    let tam = core::mem::size_of::<abi::SigFrame>() as u64;
    // Por debajo de la zona roja de 128 bytes de la ABI SysV, alineado a 16 y
    // luego 8 menos: el manejador entra como tras un `call` (`rsp % 16 == 8`),
    // con la dirección de vuelta —el restaurador— en `[rsp]`.
    let dir = p
        .ctx
        .rsp
        .checked_sub(128 + tam)
        .map(|d| (d & !0xF).wrapping_sub(8))
        .filter(|&d| d >= 4096);
    let Some(dir) = dir else {
        crate::println!("task: [{}] {} sin pila para el manejador de la señal {sig}", p.pid, p.name);
        return Entrega::Terminar(255);
    };
    if !preparar_pila(space, dir, tam)
        || super::uaccess::copy_to_space(space, dir, como_bytes(&marco)).is_err()
    {
        crate::println!(
            "task: [{}] {} sin pila para el manejador de la señal {sig} (rsp={:#x})",
            p.pid,
            p.name,
            p.ctx.rsp
        );
        return Entrega::Terminar(255);
    }
    p.ctx.rip = manejador;
    p.ctx.rdi = sig;
    p.ctx.rsp = dir;
    // Fuera DF, TF y AC: el manejador empieza como una función normal.
    p.ctx.rflags = (p.ctx.rflags & !(0x400 | 0x100 | 0x4_0000)) | 0x202;
    // Mientras corre su manejador, la misma señal espera.
    p.senales.mascara |= abi::sig_bit(sig);
    Entrega::Inyectada
}

/// Materializa las páginas del marco igual que lo haría una falta de página:
/// crecimiento de la pila principal o una región anónima escribible (la pila
/// de un hilo). No toma `PROCS` (el planificador ya lo tiene): sólo el candado
/// del mmap del espacio y el del asignador de marcos, en el mismo orden que
/// `handle_mmap_fault`.
fn preparar_pila(space: &AddrSpace, dir: u64, len: u64) -> bool {
    let fin = match dir.checked_add(len) {
        Some(f) if f <= USER_MAX => f,
        _ => return false,
    };
    let mut pagina = dir & !0xfff;
    while pagina < fin {
        if !space.is_mapped(pagina) {
            let pila_principal = pagina < STACK_TOP && pagina >= STACK_TOP - STACK_MAX;
            let anonima = space.find_mmap_region(pagina).is_some_and(|r| {
                r.inode == 0 && r.shm_id == 0 && r.writable && !r.sin_acceso
            });
            if !(pila_principal || anonima) || space.ensure_mapped(pagina).is_none() {
                return false;
            }
        }
        pagina += 4096;
    }
    space.range_ok(dir, len, true)
}

/// Lee y valida el marco de `sigreturn`. `None` = marco manipulado o ilegible.
pub fn leer_marco(space: &AddrSpace, user_rsp: u64) -> Option<(Context, crate::arch::fpu::FpuArea, u64)> {
    let dir = user_rsp.checked_sub(8)?;
    let mut marco = abi::SigFrame {
        restaurador: 0,
        sig: 0,
        mascara: 0,
        _reservado: 0,
        regs: [0; 18],
        fpu: [0; 1024],
    };
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(
            (&mut marco as *mut abi::SigFrame).cast::<u8>(),
            core::mem::size_of::<abi::SigFrame>(),
        )
    };
    super::uaccess::copy_from_space(space, dir, bytes).ok()?;
    let mut ctx = regs_a_ctx(marco.regs);
    if ctx.rip >= USER_MAX || ctx.rsp >= USER_MAX || ctx.rsp == 0 {
        return None;
    }
    if !crate::arch::fpu::imagen_valida(&marco.fpu) {
        return None;
    }
    // Sólo las banderas aritméticas (CF PF AF ZF SF OF) son del usuario; IF
    // siempre a 1 y el bit 1 reservado a 1. IOPL, NT, TF, DF, AC: fuera.
    ctx.rflags = (ctx.rflags & 0x8D5) | 0x202;
    let mut fpu = crate::arch::fpu::FpuArea::empty();
    fpu.0 = marco.fpu;
    Some((ctx, fpu, marco.mascara & !abi::SENALES_INCAPTURABLES))
}

/// ¿Está este proceso bloqueado en algo que una señal deba interrumpir?
/// Si lo está, lo despierta con `EINTR`.
pub fn interrumpir_bloqueo(p: &mut Process) {
    let interrumpible = match p.state {
        State::Sleeping(_)
        | State::WaitingTty { .. }
        | State::WaitingPipe { .. }
        | State::WaitingFutex { .. }
        | State::WaitingEventos { .. }
        | State::WaitingChild { .. } => true,
        State::WaitingSocket {
            connect, result_fd, ..
        } => {
            if connect {
                super::syscall::close_fd_slot(p, result_fd);
            }
            true
        }
        _ => false,
    };
    if interrumpible {
        p.ctx.rax = (-abi::EINTR) as u64;
        p.state = State::Runnable;
    }
}

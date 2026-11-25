//! `SYS_ESPERAR`: dormir hasta que un conjunto de fuentes tenga algo que leer.
//!
//! Lo usa el escritorio (soso-wm) para no girar cada 10 ms: espera a la vez a
//! la entrada, al listener, a cada cliente y a cada PTY. Las fuentes se
//! resuelven al entrar (de fd a slot) y el planificador las vuelve a mirar en
//! cada pasada (`despertar`), igual que los demás estados de espera.

use alloc::vec::Vec;
use soso_abi as abi;

use super::{Console, Fd, Process, State};

/// Una entrada de la lista, ya resuelta.
#[derive(Clone, Copy, Debug)]
pub enum Fuente {
    Entrada,
    Tcp(usize),
    Tuberia(super::pipe::PipeId),
    Pty(u8),
    /// Fd cerrado o de un tipo que no se espera: cuenta como listo (como
    /// `POLLNVAL`), para que quien espera se entere en vez de dormir de más.
    Invalida,
}

pub fn resolver(p: &Process, entrada: u32) -> Fuente {
    if entrada == abi::ESPERAR_ENTRADA {
        return Fuente::Entrada;
    }
    match p.fds.get(entrada as usize).and_then(|f| f.as_ref()) {
        Some(Fd::Tcp { slot }) => Fuente::Tcp(*slot),
        Some(Fd::PipeRead(id)) => Fuente::Tuberia(*id),
        Some(Fd::PtyMaster(slot)) => Fuente::Pty(*slot),
        _ => Fuente::Invalida,
    }
}

/// Nadie vivo usa ya el PTY como consola: el otro extremo colgó.
fn pty_colgado(procs: &[Process], slot: u8) -> bool {
    !procs
        .iter()
        .any(|q| q.console == Console::Pty(slot) && !matches!(q.state, State::Zombie(_)))
}

pub fn lista(f: Fuente, procs: &[Process]) -> bool {
    match f {
        Fuente::Entrada => crate::drivers::input::hay_eventos(),
        Fuente::Tcp(slot) => crate::net::tcp_legible(slot),
        Fuente::Tuberia(id) => super::pipe::has_data(id) || super::pipe::write_closed(id),
        Fuente::Pty(slot) => super::pty::master_legible(slot) || pty_colgado(procs, slot),
        Fuente::Invalida => true,
    }
}

/// Entradas con la marca de listas puesta, y cuántas lo están.
pub fn marcar(fuentes: &[(u32, Fuente)], procs: &[Process]) -> (Vec<u8>, u64) {
    let mut out = Vec::with_capacity(fuentes.len() * 4);
    let mut n = 0u64;
    for &(entrada, f) in fuentes {
        let mut v = entrada & !abi::ESPERAR_LISTO;
        if lista(f, procs) {
            v |= abi::ESPERAR_LISTO;
            n += 1;
        }
        out.extend_from_slice(&v.to_le_bytes());
    }
    (out, n)
}

/// Pasada del planificador: despierta a quien tenga algo listo o el plazo
/// vencido, con la lista marcada en su memoria y el recuento en `rax`. Se llama
/// con `PROCS` cogido.
pub fn despertar(procs: &mut [Process], now: u64) {
    for i in 0..procs.len() {
        let State::WaitingEventos { lista: ptr, deadline_ms } = procs[i].state else {
            continue;
        };
        let (marcada, n) = marcar(&procs[i].espera, procs);
        let vencido = deadline_ms != 0 && now >= deadline_ms;
        if n == 0 && !vencido {
            continue;
        }
        let p = &mut procs[i];
        let ok = p.space.as_ref().is_some_and(|s| {
            s.activate();
            s.write(ptr, &marcada).is_some()
        });
        p.ctx.rax = if ok { n } else { (-abi::EFAULT) as u64 };
        p.espera.clear();
        p.state = State::Runnable;
    }
}

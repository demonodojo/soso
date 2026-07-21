//! Entrada de syscalls (instrucción `syscall` → MSRs STAR/LSTAR/SFMASK)
//! y despacho de las 14 llamadas de soso.
//!
//! Contrato con el usuario (ver soso-abi): nº en rax, args en
//! rdi/rsi/rdx/r10, retorno en rax (negativo = -errno). El kernel preserva
//! rsp y los callee-saved; el resto quedan clobber.

use super::addrspace::{BRK_MAX, USER_MAX};
use super::pipe;
use super::{Context, Fd, MAX_FDS, State};
use crate::arch::gdt;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::naked_asm;
use soso_abi as abi;
use sosofs::FsError;
use x86_64::VirtAddr;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};
use x86_64::registers::rflags::RFlags;
use x86_64::structures::paging::PageTableFlags;

pub fn init_msrs() {
    let s = gdt::selectors();
    unsafe {
        Efer::update(|f| f.insert(EferFlags::SYSTEM_CALL_EXTENSIONS));
        Star::write(s.ucode, s.udata, s.kcode, s.kdata).expect("layout GDT inválido para STAR");
        LStar::write(VirtAddr::new(syscall_entry as *const () as u64));
        // La CPU limpia estos flags al entrar: interrupciones fuera hasta
        // que la pila de kernel esté puesta.
        SFMask::write(RFlags::INTERRUPT_FLAG | RFlags::TRAP_FLAG | RFlags::DIRECTION_FLAG);
    }
}

/// Como `init_msrs` pero para un AP: STAR/SFMASK son iguales (mismo GDT
/// compartido), solo cambia LSTAR — apunta a `ap_syscall_entry`, que usa la
/// pila/scratch de ESTE core vía GS en vez de los símbolos fijos de la BSP
/// (`gdt::KSTACK`/`USER_RSP_SCRATCH`, que reventarían si dos cores
/// syscalleasen a la vez).
pub fn init_msrs_ap() {
    let s = gdt::selectors();
    unsafe {
        Efer::update(|f| f.insert(EferFlags::SYSTEM_CALL_EXTENSIONS));
        Star::write(s.ucode, s.udata, s.kcode, s.kdata).expect("layout GDT inválido para STAR");
        LStar::write(VirtAddr::new(ap_syscall_entry as *const () as u64));
        SFMask::write(RFlags::INTERRUPT_FLAG | RFlags::TRAP_FLAG | RFlags::DIRECTION_FLAG);
    }
}

/// Marco que syscall_entry deja en la pila (campos en orden de memoria
/// ascendente = push inverso).
#[repr(C)]
pub struct SyscallFrame {
    pub r10: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub nr: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub user_rsp: u64,
    pub rflags: u64,
    pub rip: u64,
}

static mut USER_RSP_SCRATCH: u64 = 0;

#[unsafe(naked)]
extern "C" fn syscall_entry() {
    naked_asm!(
        // rcx = rip de usuario, r11 = rflags; rsp aún es el del usuario.
        "mov [rip + {scratch}], rsp",
        "lea rsp, [rip + {kstack}]",
        "add rsp, {size}",
        "push rcx",
        "push r11",
        "push [rip + {scratch}]",
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rax",
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "mov rdi, rsp",
        // 14 pushes (112 B) desde el tope alineado dejan rsp%16==0 antes
        // del call, que es lo que la ABI pide (dispatch entra con %16==8).
        "sti", // con la pila puesta ya pueden entrar ticks
        "call {dispatch}",
        "cli",
        "add rsp, 5 * 8", // args + nr
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "pop rdx", // rsp de usuario (rdx es clobber del ABI de syscall)
        "pop r11",
        "pop rcx",
        "mov rsp, rdx",
        "sysretq",
        scratch = sym USER_RSP_SCRATCH,
        kstack = sym gdt::KSTACK,
        size = const gdt::KSTACK_SIZE,
        dispatch = sym dispatch,
    )
}

/// Como `syscall_entry` pero para un AP: el rsp de usuario y la pila de
/// kernel se leen de los campos por-CPU (GS), no de `USER_RSP_SCRATCH`/
/// `gdt::KSTACK` — dos cores en syscall a la vez no pueden pisarse. El
/// cuerpo (`dispatch`) es el mismo: `SyscallFrame` tiene idéntico layout.
#[unsafe(naked)]
extern "C" fn ap_syscall_entry() {
    naked_asm!(
        "mov gs:[{scratch}], rsp",
        "mov rsp, gs:[{kstack}]",
        "push rcx",
        "push r11",
        "push gs:[{scratch}]",
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rax",
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "mov rdi, rsp",
        "sti",
        "call {dispatch}",
        "cli",
        "add rsp, 5 * 8",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "pop rdx",
        "pop r11",
        "pop rcx",
        "mov rsp, rdx",
        "sysretq",
        scratch = const crate::arch::percpu::OFF_SYSCALL_SCRATCH,
        kstack = const crate::arch::percpu::OFF_KSTACK_TOP,
        dispatch = sym dispatch,
    )
}

/// Contexto para reanudar tras una syscall bloqueante: los caller-saved
/// son clobber del ABI, solo persisten los callee-saved y rip/rsp/rflags.
fn ctx_from_frame(f: &SyscallFrame) -> Context {
    Context {
        r15: f.r15,
        r14: f.r14,
        r13: f.r13,
        r12: f.r12,
        rbp: f.rbp,
        rbx: f.rbx,
        rip: f.rip,
        rsp: f.user_rsp,
        rflags: f.rflags,
        ..Context::default()
    }
}

extern "C" fn dispatch(f: &mut SyscallFrame) -> i64 {
    let (a1, a2, a3, a4) = (f.rdi, f.rsi, f.rdx, f.r10);
    let r = match f.nr {
        abi::SYS_EXIT => super::exit_current(a1 as u8),
        abi::SYS_READ => sys_read(f, a1, a2, a3),
        abi::SYS_WRITE => sys_write(f, a1, a2, a3),
        abi::SYS_OPEN => sys_open(a1, a2, a3),
        abi::SYS_CLOSE => sys_close(a1),
        abi::SYS_SEEK => sys_seek(a1, a2 as i64, a3),
        abi::SYS_STAT => sys_stat(a1, a2, a3),
        abi::SYS_GETDENTS => sys_getdents(a1, a2, a3),
        abi::SYS_MKDIR => sys_mkdir(a1, a2),
        abi::SYS_UNLINK => sys_unlink(a1, a2),
        abi::SYS_SPAWN => sys_spawn(a1, a2, a3, a4),
        abi::SYS_WAIT => sys_wait(f),
        abi::SYS_SBRK => sys_sbrk(a1 as i64),
        abi::SYS_SLEEP_MS => super::block_current(
            ctx_from_frame(f),
            State::Sleeping(crate::arch::pit::uptime_ms() + a1),
        ),
        abi::SYS_HALT => {
            crate::println!("halt: apagando soso");
            crate::qemu::exit(crate::qemu::ExitCode::Success);
        }
        abi::SYS_MMAP => sys_mmap(a1, a2, a3, a4),
        abi::SYS_MUNMAP => sys_munmap(a1, a2),
        abi::SYS_GPU_INFO => sys_gpu_info(a1),
        abi::SYS_GPU_ALLOC => sys_gpu_alloc(a1),
        abi::SYS_GPU_MAP => sys_gpu_map(a1, a2, a3),
        abi::SYS_GPU_SUBMIT => sys_gpu_submit(a1, a2),
        abi::SYS_PIPE => sys_pipe(),
        abi::SYS_SPAWN_IO => sys_spawn_io(a1),
        abi::SYS_CHDIR => sys_chdir(a1, a2),
        abi::SYS_GETCWD => sys_getcwd(a1, a2),
        abi::SYS_THREAD_SPAWN => sys_thread_spawn(a1, a2, a3),
        abi::SYS_FUTEX => sys_futex(f, a1, a2, a3, a4),
        abi::SYS_NCPU => Ok(crate::arch::smp::CPUS_ONLINE.load(
            core::sync::atomic::Ordering::Relaxed,
        ) as u64),
        _ => Err(-abi::ENOSYS),
    };
    match r {
        Ok(v) => v as i64,
        Err(e) => e,
    }
}

// ---- acceso a memoria de usuario ----
// CR3 es el del proceso durante toda la syscall: validado el rango, se
// puede desreferenciar directamente.

fn user_range_ok(ptr: u64, len: u64, need_write: bool) -> bool {
    if len == 0 {
        return true;
    }
    if ptr == 0 || len > 16 * 1024 * 1024 || ptr.checked_add(len).is_none_or(|e| e > USER_MAX) {
        return false;
    }
    super::with_current(|p| {
        let space = p.space.as_ref().unwrap();
        let mut page = ptr & !0xfff;
        while page < ptr + len {
            match space.translate_flags(page) {
                Some(fl)
                    if fl.contains(PageTableFlags::USER_ACCESSIBLE)
                        && (!need_write || fl.contains(PageTableFlags::WRITABLE)) => {}
                _ => return false,
            }
            page += 4096;
        }
        true
    })
}

fn user_slice(ptr: u64, len: u64) -> Result<&'static [u8], i64> {
    if !user_range_ok(ptr, len, false) {
        return Err(-abi::EFAULT);
    }
    Ok(unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) })
}

fn user_slice_mut(ptr: u64, len: u64) -> Result<&'static mut [u8], i64> {
    if !user_range_ok(ptr, len, true) {
        return Err(-abi::EFAULT);
    }
    Ok(unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize) })
}

fn user_str(ptr: u64, len: u64) -> Result<&'static str, i64> {
    if len > 4096 {
        return Err(-abi::ENAMETOOLONG);
    }
    core::str::from_utf8(user_slice(ptr, len)?).map_err(|_| -abi::EINVAL)
}

// ---- helpers de VFS ----

pub fn fs_errno(e: FsError) -> i64 {
    -match e {
        FsError::NotFound => abi::ENOENT,
        FsError::NotADir => abi::ENOTDIR,
        FsError::NotAFile => abi::EISDIR,
        FsError::NameTooLong => abi::ENAMETOOLONG,
        FsError::NoSpace => abi::ENOSPC,
        FsError::Exists => abi::EEXIST,
        FsError::DirNotEmpty => abi::ENOTEMPTY,
        _ => abi::EIO,
    }
}

fn with_vfs<R>(f: impl FnOnce() -> Result<R, FsError>) -> Result<R, i64> {
    f().map_err(fs_errno)
}

/// Ruta de usuario resuelta contra el cwd del proceso actual.
fn resolve_user_path(path_ptr: u64, path_len: u64) -> Result<String, i64> {
    let raw = user_str(path_ptr, path_len)?;
    let cwd = super::with_current(|p| p.cwd.clone());
    super::path::abs_path(&cwd, raw)
}

/// Separa una ruta absoluta en (inode del padre, nombre final).
fn resolve_parent(abs: &str) -> Result<(u64, String), i64> {
    let path = abs.trim_end_matches('/');
    if path.is_empty() || path == "." {
        return Err(-abi::EINVAL);
    }
    let (parent, name) = match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(i) => (&path[..i], &path[i + 1..]),
        None => return Err(-abi::EINVAL),
    };
    if name.is_empty() || name == "." {
        return Err(-abi::EINVAL);
    }
    let dir = with_vfs(|| crate::vfs::resolve(parent))?;
    Ok((dir, String::from(name)))
}

fn with_fd<R>(fd: u64, f: impl FnOnce(&mut Fd) -> Result<R, i64>) -> Result<R, i64> {
    super::with_current(|p| {
        let slot = p
            .fds
            .get_mut(fd as usize)
            .and_then(|s| s.as_mut())
            .ok_or(-abi::EBADF)?;
        f(slot)
    })
}

fn alloc_fd(p: &mut super::Process, fd: Fd) -> Result<u64, i64> {
    match p.fds.iter().position(|s| s.is_none()) {
        Some(i) => {
            p.fds[i] = Some(fd);
            Ok(i as u64)
        }
        None if p.fds.len() < MAX_FDS => {
            p.fds.push(Some(fd));
            Ok((p.fds.len() - 1) as u64)
        }
        None => Err(-abi::EMFILE),
    }
}

fn fd_ok_for_stdin(fd: &Fd) -> bool {
    matches!(
        fd,
        Fd::Tty | Fd::File { .. } | Fd::LazyFile { .. } | Fd::PipeRead(_)
    )
}

fn fd_ok_for_stdout(fd: &Fd) -> bool {
    matches!(fd, Fd::Tty | Fd::WriteBuf { .. } | Fd::PipeWrite(_))
}

/// Transfiere fds del padre al hijo según `stdio` (FD_INHERIT_TTY = tty).
pub fn take_stdio_fds(stdio: [u64; 3]) -> Result<[Option<Fd>; 3], i64> {
    super::with_current(|p| {
        for (slot, &spec) in stdio.iter().enumerate() {
            if spec == abi::FD_INHERIT_TTY {
                continue;
            }
            let fd = p
                .fds
                .get(spec as usize)
                .and_then(|s| s.as_ref())
                .ok_or(-abi::EBADF)?;
            let ok = if slot == 0 {
                fd_ok_for_stdin(fd)
            } else {
                fd_ok_for_stdout(fd)
            };
            if !ok {
                return Err(-abi::EBADF);
            }
        }
        let mut out: [Option<Fd>; 3] = [None, None, None];
        for (slot, spec) in stdio.into_iter().enumerate() {
            if spec == abi::FD_INHERIT_TTY {
                continue;
            }
            out[slot] = p.fds.get_mut(spec as usize).and_then(|s| s.take());
        }
        Ok(out)
    })
}

/// Cierra un fd y aplica efectos secundarios (commit, pipes).
pub fn drop_fd(fd: Fd) -> Result<(), i64> {
    match fd {
        Fd::WriteBuf { dir, name, data, .. } => {
            let mtime = crate::arch::pit::uptime_ms() / 1000;
            with_vfs(|| crate::vfs::create_file(dir, &name, &data, mtime))?;
        }
        Fd::PipeRead(id) => pipe::close_reader(id),
        Fd::PipeWrite(id) => pipe::close_writer(id),
        _ => {}
    }
    Ok(())
}

/// Cierra todos los fds abiertos de un proceso (p. ej. al hacer exit).
pub fn close_all_fds(fds: &mut Vec<Option<Fd>>) {
    for slot in fds.iter_mut() {
        if let Some(fd) = slot.take() {
            let _ = drop_fd(fd);
        }
    }
}

// ---- las syscalls ----

fn sys_write(f: &mut SyscallFrame, fd: u64, buf: u64, len: u64) -> Result<u64, i64> {
    let pipe_id = with_fd(fd, |slot| {
        Ok(match slot {
            Fd::PipeWrite(id) => Some(*id),
            _ => None,
        })
    })?;
    if let Some(id) = pipe_id {
        if len == 0 {
            return Ok(0);
        }
        if pipe::no_readers(id) {
            return Err(-abi::EPIPE);
        }
        let n = pipe::try_write(id, buf, len)?;
        if n > 0 {
            return Ok(n);
        }
        if pipe::no_readers(id) {
            return Err(-abi::EPIPE);
        }
        super::block_current(
            ctx_from_frame(f),
            State::WaitingPipe {
                pipe_id: id,
                buf,
                len,
                write: true,
            },
        );
    }
    let data = user_slice(buf, len)?;
    // La consola se lee fuera de with_fd (que ya tiene tomado PROCS).
    let console = super::with_current(|p| p.console);
    with_fd(fd, |f| match f {
        Fd::Tty => {
            console.write_bytes(data);
            Ok(len)
        }
        Fd::WriteBuf { data: out, pos, .. } => {
            if *pos + data.len() > 16 * 1024 * 1024 {
                return Err(-abi::ENOSPC);
            }
            if *pos + data.len() > out.len() {
                out.resize(*pos + data.len(), 0);
            }
            out[*pos..*pos + data.len()].copy_from_slice(data);
            *pos += data.len();
            Ok(len)
        }
        _ => Err(-abi::EBADF),
    })
}

fn sys_read(f: &mut SyscallFrame, fd: u64, buf: u64, len: u64) -> Result<u64, i64> {
    let dst = user_slice_mut(buf, len)?;
    let pipe_id = with_fd(fd, |slot| {
        Ok(match slot {
            Fd::PipeRead(id) => Some(*id),
            _ => None,
        })
    })?;
    if let Some(id) = pipe_id {
        if len == 0 {
            return Ok(0);
        }
        let n = pipe::try_read(id, buf, len);
        if n > 0 {
            return Ok(n);
        }
        if pipe::write_closed(id) {
            return Ok(0);
        }
        super::block_current(
            ctx_from_frame(f),
            State::WaitingPipe {
                pipe_id: id,
                buf,
                len,
                write: false,
            },
        );
    }
    // ¿Es la tty? El caso bloqueante no puede resolverse dentro de with_fd
    // (block_current no retorna), así que se distingue antes.
    let es_tty = with_fd(fd, |f| Ok(matches!(f, Fd::Tty)))?;
    if es_tty {
        if len == 0 {
            return Ok(0);
        }
        let console = super::with_current(|p| p.console);
        let mut n = 0usize;
        while n < dst.len() {
            match console.read_byte() {
                Some(b) => {
                    dst[n] = b;
                    n += 1;
                }
                None => break,
            }
        }
        if n > 0 {
            return Ok(n as u64);
        }
        // Sin datos: a dormir hasta que lleguen (el scheduler hace la copia).
        super::block_current(ctx_from_frame(f), State::WaitingTty { buf, len });
    }
    with_fd(fd, |f| match f {
        Fd::File { data, pos, .. } => {
            let n = dst.len().min(data.len().saturating_sub(*pos));
            dst[..n].copy_from_slice(&data[*pos..*pos + n]);
            *pos += n;
            Ok(n as u64)
        }
        Fd::LazyFile { inode, size, pos } => {
            let n = dst.len().min(size.saturating_sub(*pos));
            if n == 0 {
                return Ok(0);
            }
            with_vfs(|| crate::vfs::read_file_range(*inode, *pos, n, &mut dst[..n]))?;
            *pos += n;
            Ok(n as u64)
        }
        Fd::Dir { .. } => Err(-abi::EISDIR),
        _ => Err(-abi::EBADF),
    })
}

fn sys_open(path_ptr: u64, path_len: u64, flags: u64) -> Result<u64, i64> {
    let path = resolve_user_path(path_ptr, path_len)?;
    let nuevo = if flags & abi::O_WRONLY != 0 {
        let (dir, name) = resolve_parent(&path)?;
        // Si existe y es un directorio, no se puede sobreescribir.
        if let Ok(ino) = with_vfs(|| crate::vfs::lookup(dir, &name))
            && with_vfs(|| crate::vfs::stat_inode(ino))?.file_type == sosofs::layout::FT_DIR
        {
            return Err(-abi::EISDIR);
        }
        let mut data = Vec::new();
        if flags & abi::O_APPEND != 0
            && let Ok(ino) = with_vfs(|| crate::vfs::lookup(dir, &name))
            && with_vfs(|| crate::vfs::stat_inode(ino))?.file_type == sosofs::layout::FT_FILE
        {
            data = with_vfs(|| crate::vfs::read_file(ino))?;
        }
        let pos = data.len();
        Fd::WriteBuf { dir, name, data, pos }
    } else {
        let ino = with_vfs(|| crate::vfs::resolve(&path))?;
        let st = with_vfs(|| crate::vfs::stat_inode(ino))?;
        if st.file_type == sosofs::layout::FT_DIR {
            let entries = with_vfs(|| crate::vfs::read_dir(ino))?
                .into_iter()
                .map(|(name, child)| {
                    let st = with_vfs(|| crate::vfs::stat_inode(child))?;
                    let mut d = abi::Dirent {
                        ino: child,
                        file_type: st.file_type,
                        name_len: name.len().min(abi::NAME_MAX) as u8,
                        ..abi::Dirent::default()
                    };
                    let n = name.len().min(abi::NAME_MAX);
                    d.name[..n].copy_from_slice(&name.as_bytes()[..n]);
                    Ok(d)
                })
                .collect::<Result<Vec<_>, i64>>()?;
            Fd::Dir { entries, pos: 0 }
        } else {
            let st = with_vfs(|| crate::vfs::stat_inode(ino))?;
            if st.size.get() > abi::LAZY_FILE_THRESHOLD {
                Fd::LazyFile {
                    inode: ino,
                    size: st.size.get() as usize,
                    pos: 0,
                }
            } else {
                let data = with_vfs(|| crate::vfs::read_file(ino))?;
                Fd::File { inode: ino, data, pos: 0 }
            }
        }
    };
    super::with_current(|p| alloc_fd(p, nuevo))
}

fn sys_close(fd: u64) -> Result<u64, i64> {
    let cerrado = super::with_current(|p| {
        p.fds
            .get_mut(fd as usize)
            .and_then(|s| s.take())
            .ok_or(-abi::EBADF)
    })?;
    drop_fd(cerrado)?;
    Ok(0)
}

fn sys_seek(fd: u64, off: i64, whence: u64) -> Result<u64, i64> {
    with_fd(fd, |f| {
        let (pos, len) = match f {
            Fd::File { data, pos, .. } => (pos, data.len()),
            Fd::LazyFile { size, pos, .. } => (pos, *size),
            Fd::WriteBuf { data, pos, .. } => (pos, data.len()),
            _ => return Err(-abi::ESPIPE),
        };
        let base = match whence {
            abi::SEEK_SET => 0,
            abi::SEEK_CUR => *pos as i64,
            abi::SEEK_END => len as i64,
            _ => return Err(-abi::EINVAL),
        };
        let nuevo = base + off;
        if nuevo < 0 {
            return Err(-abi::EINVAL);
        }
        *pos = nuevo as usize;
        Ok(*pos as u64)
    })
}

fn sys_stat(path_ptr: u64, path_len: u64, out: u64) -> Result<u64, i64> {
    let path = resolve_user_path(path_ptr, path_len)?;
    let dst = user_slice_mut(out, core::mem::size_of::<abi::Stat>() as u64)?;
    let (ino, st) = with_vfs(|| {
        let ino = crate::vfs::resolve(&path)?;
        Ok((ino, crate::vfs::stat_inode(ino)?))
    })?;
    let stat = abi::Stat {
        ino,
        size: st.size.get(),
        mtime: st.mtime.get(),
        file_type: st.file_type,
        _pad: [0; 7],
    };
    dst.copy_from_slice(unsafe {
        core::slice::from_raw_parts(
            (&stat as *const abi::Stat).cast::<u8>(),
            core::mem::size_of::<abi::Stat>(),
        )
    });
    Ok(0)
}

fn sys_getdents(fd: u64, buf: u64, len: u64) -> Result<u64, i64> {
    let dst = user_slice_mut(buf, len)?;
    with_fd(fd, |f| {
        let Fd::Dir { entries, pos } = f else { return Err(-abi::ENOTDIR) };
        let mut escrito = 0usize;
        while *pos < entries.len() && escrito + abi::DIRENT_SIZE <= dst.len() {
            let d = &entries[*pos];
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (d as *const abi::Dirent).cast::<u8>(),
                    abi::DIRENT_SIZE,
                )
            };
            dst[escrito..escrito + abi::DIRENT_SIZE].copy_from_slice(bytes);
            escrito += abi::DIRENT_SIZE;
            *pos += 1;
        }
        Ok(escrito as u64)
    })
}

fn sys_mkdir(path_ptr: u64, path_len: u64) -> Result<u64, i64> {
    let path = resolve_user_path(path_ptr, path_len)?;
    let (dir, name) = resolve_parent(&path)?;
    let mtime = crate::arch::pit::uptime_ms() / 1000;
    with_vfs(|| crate::vfs::mkdir(dir, &name, mtime))?;
    Ok(0)
}

fn sys_unlink(path_ptr: u64, path_len: u64) -> Result<u64, i64> {
    let path = resolve_user_path(path_ptr, path_len)?;
    let (dir, name) = resolve_parent(&path)?;
    with_vfs(|| crate::vfs::unlink(dir, &name))?;
    Ok(0)
}

fn sys_chdir(path_ptr: u64, path_len: u64) -> Result<u64, i64> {
    let path = resolve_user_path(path_ptr, path_len)?;
    let ino = with_vfs(|| crate::vfs::resolve(&path))?;
    let st = with_vfs(|| crate::vfs::stat_inode(ino))?;
    if st.file_type != sosofs::layout::FT_DIR {
        return Err(-abi::ENOTDIR);
    }
    super::with_current(|p| p.cwd = path);
    Ok(0)
}

fn sys_getcwd(buf_ptr: u64, len: u64) -> Result<u64, i64> {
    let cwd = super::with_current(|p| p.cwd.clone());
    let bytes = cwd.as_bytes();
    if len < bytes.len() as u64 + 1 {
        return Err(-abi::EINVAL);
    }
    let buf = user_slice_mut(buf_ptr, len)?;
    buf[..bytes.len()].copy_from_slice(bytes);
    buf[bytes.len()] = 0;
    Ok(buf_ptr)
}

fn sys_spawn(path_ptr: u64, path_len: u64, args_ptr: u64, args_len: u64) -> Result<u64, i64> {
    let path = user_str(path_ptr, path_len)?;
    let args = if args_len == 0 { "" } else { user_str(args_ptr, args_len)? };
    // El hijo hereda la consola del padre: si la shell corre por SSH, sus
    // coreutils deben escribir al canal SSH, no al puerto serie.
    let console = super::with_current(|p| p.console);
    super::spawn_console(path, args, super::current_pid(), console)
}

fn sys_spawn_io(opts_ptr: u64) -> Result<u64, i64> {
    let opts = user_slice(opts_ptr, core::mem::size_of::<abi::SpawnIo>() as u64)?;
    let opts = unsafe { *(opts.as_ptr() as *const abi::SpawnIo) };
    let path = user_str(opts.path_ptr, opts.path_len)?;
    let args = if opts.args_len == 0 {
        ""
    } else {
        user_str(opts.args_ptr, opts.args_len)?
    };
    let console = super::with_current(|p| p.console);
    super::spawn_console_io(
        path,
        args,
        super::current_pid(),
        console,
        [opts.stdin_fd, opts.stdout_fd, opts.stderr_fd],
    )
}

fn sys_pipe() -> Result<u64, i64> {
    let id = pipe::alloc_pipe();
    pipe::add_reader(id);
    pipe::add_writer(id);
    super::with_current(|p| {
        let read_fd = alloc_fd(p, Fd::PipeRead(id))?;
        let write_fd = alloc_fd(p, Fd::PipeWrite(id))?;
        Ok((write_fd << 32) | read_fd)
    })
}

fn sys_wait(f: &mut SyscallFrame) -> Result<u64, i64> {
    let pid = super::current_pid();
    let listo = {
        let mut procs = super::PROCS.lock();
        if !procs.iter().any(|p| p.parent == pid) {
            return Err(-abi::ECHILD);
        }
        match procs
            .iter()
            .position(|p| p.parent == pid && matches!(p.state, State::Zombie(_)))
        {
            Some(i) => {
                let hijo = procs.remove(i);
                let State::Zombie(code) = hijo.state else { unreachable!() };
                Some(super::wait_pack(hijo.pid, code))
            }
            None => None,
        }
    };
    match listo {
        Some(v) => Ok(v),
        // Sin zombis todavía: a dormir; exit() del hijo pone el rax.
        None => super::block_current(ctx_from_frame(f), State::WaitingChild),
    }
}

fn sys_sbrk(delta: i64) -> Result<u64, i64> {
    super::with_current(|p| {
        let old = p.brk;
        let nuevo = old
            .checked_add_signed(delta)
            .filter(|&b| b >= p.brk_min && b <= BRK_MAX)
            .ok_or(-abi::ENOMEM)?;
        if nuevo > old {
            let space = p.space.as_ref().unwrap().clone();
            for va in (old.next_multiple_of(4096)..nuevo.next_multiple_of(4096)).step_by(4096)
            {
                space.ensure_mapped(va).ok_or(-abi::ENOMEM)?;
            }
        }
        p.brk = nuevo;
        Ok(old)
    })
}

fn sys_mmap(addr: u64, len: u64, fd: u64, offset: u64) -> Result<u64, i64> {
    if len == 0 {
        return Err(-abi::EINVAL);
    }
    let len_aligned = len.checked_next_multiple_of(4096).ok_or(-abi::EINVAL)?;
    let (inode, file_len, writable) = if fd == u64::MAX {
        (0u64, 0u64, true)
    } else {
        let (ino, fsize) = super::with_current(|p| {
            let slot = p
                .fds
                .get(fd as usize)
                .and_then(|s| s.as_ref())
                .ok_or(-abi::EBADF)?;
            match slot {
                Fd::File { inode, data, .. } => Ok((*inode, data.len() as u64)),
                Fd::LazyFile { inode, size, .. } => Ok((*inode, *size as u64)),
                _ => Err(-abi::EBADF),
            }
        })?;
        let end = offset.checked_add(len).ok_or(-abi::EINVAL)?;
        if offset >= fsize || end > fsize {
            return Err(-abi::EINVAL);
        }
        (ino, fsize, false)
    };
    super::with_current(|p| {
        let space = p.space.as_ref().ok_or(-abi::ENOMEM)?;
        let virt = space.with_mmap_mut(|book| {
            let virt = super::mmap::next_addr(&book.regions, addr, len_aligned).ok_or(-abi::ENOMEM)?;
            book.regions.push(super::mmap::MmapRegion {
                virt_start: virt,
                len: len_aligned,
                inode,
                file_offset: offset,
                file_len,
                writable,
            });
            if virt >= book.next {
                book.next = virt + len_aligned;
            }
            Ok::<u64, i64>(virt)
        })?;
        Ok(virt)
    })
}

fn sys_munmap(addr: u64, len: u64) -> Result<u64, i64> {
    if len == 0 {
        return Err(-abi::EINVAL);
    }
    let len_aligned = len.next_multiple_of(4096);
    super::with_current(|p| {
        let space = p.space.as_ref().ok_or(-abi::ENOMEM)?;
        let ok = space.with_mmap_mut(|book| {
            super::mmap::remove_region(&mut book.regions, addr, len_aligned)
        });
        if !ok {
            return Err(-abi::EINVAL);
        }
        space.unmap_range(addr, len_aligned);
        Ok(0)
    })
}

fn sys_gpu_info(out: u64) -> Result<u64, i64> {
    let dst = user_slice_mut(out, core::mem::size_of::<abi::GpuInfo>() as u64)?;
    let info = crate::drivers::gpu::info();
    dst.copy_from_slice(unsafe {
        core::slice::from_raw_parts(
            (&info as *const abi::GpuInfo).cast::<u8>(),
            core::mem::size_of::<abi::GpuInfo>(),
        )
    });
    Ok(0)
}

fn sys_gpu_alloc(size: u64) -> Result<u64, i64> {
    crate::drivers::gpu::alloc(size).map_err(|e| -e)
}

fn sys_gpu_map(gpu_handle: u64, user_ptr: u64, len: u64) -> Result<u64, i64> {
    if !user_range_ok(user_ptr, len, true) {
        return Err(-abi::EFAULT);
    }
    crate::drivers::gpu::map_to_user(gpu_handle, user_ptr, len).map_err(|e| -e)
}

fn sys_gpu_submit(cmd_ptr: u64, cmd_len: u64) -> Result<u64, i64> {
    let cmd = user_slice(cmd_ptr, cmd_len)?;
    crate::drivers::gpu::submit(cmd).map_err(|e| -e)
}

fn sys_thread_spawn(entry: u64, arg: u64, stack_top: u64) -> Result<u64, i64> {
    // La pila suele ser mmap anónimo (fault bajo demanda): no exigir PTE
    // presente, solo región mmap escribible o páginas ya mapeadas.
    let stack_lo = stack_top.saturating_sub(16);
    if stack_lo == 0 || stack_top > USER_MAX || entry == 0 || entry >= USER_MAX {
        return Err(-abi::EFAULT);
    }
    let stack_ok = super::with_current(|p| {
        let space = p.space.as_ref().ok_or(-abi::ENOMEM)?;
        if let Some(r) = space.find_mmap_region(stack_lo) {
            if r.writable
                && stack_lo >= r.virt_start
                && stack_top <= r.virt_start.saturating_add(r.len)
            {
                return Ok(());
            }
            return Err(-abi::EFAULT);
        }
        // Sin región mmap: exigir PTE presente y escribible (sin reentrar
        // en with_current: ya tenemos el space).
        use x86_64::structures::paging::PageTableFlags;
        match space.translate_flags(stack_lo) {
            Some(fl)
                if fl.contains(PageTableFlags::USER_ACCESSIBLE)
                    && fl.contains(PageTableFlags::WRITABLE) =>
            {
                Ok(())
            }
            _ => Err(-abi::EFAULT),
        }
    });
    stack_ok?;
    if !user_range_ok(entry, 1, false) {
        return Err(-abi::EFAULT);
    }
    super::thread_spawn(entry, arg, stack_top)
}

fn sys_futex(
    f: &mut SyscallFrame,
    op: u64,
    uaddr: u64,
    val: u64,
    nwake: u64,
) -> Result<u64, i64> {
    if !user_range_ok(uaddr, 4, true) {
        return Err(-abi::EFAULT);
    }
    let pml4 = super::with_current(|p| {
        p.space
            .as_ref()
            .map(|s| s.pml4_phys())
            .ok_or(-abi::ENOMEM)
    })?;
    match op {
        abi::FUTEX_WAIT => {
            // No vuelve: wait_or_resume bloquea o replanifica.
            super::futex::wait_or_resume(pml4, uaddr, val as u32, ctx_from_frame(f));
        }
        abi::FUTEX_WAKE => Ok(super::futex::wake(pml4, uaddr, nwake)),
        _ => Err(-abi::EINVAL),
    }
}

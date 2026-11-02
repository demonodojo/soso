//! Parches de `library/std` generados a partir de `config/rust-soso/apply-patches.sh`.
//!
//! Cada paso es el diff neto entre la revisión fijada y lo que deja el script.
//! `receta::pasos_libstd` los añade; la prueba `receta` comprueba que script y
//! receta dejan los mismos bytes.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::receta::{Accion, Paso};

pub(crate) fn pasos() -> Vec<Paso> {
    vec![
        Paso {
            nombre: String::from("sys/thread: rama soso"),
            fichero: String::from("library/std/src/sys/thread/mod.rs"),
            marca: String::from(r#"target_os = "soso" => {"#),
            accion: Accion::Cambios {
                cambios: vec![
                    (
                        String::from(r#"        mod unsupported;
        pub use unsupported::{current_os_id, set_name};
    }
    _ => {
        mod unsupported;
        pub use unsupported::{
"#),
                        String::from(r#"        mod unsupported;
        pub use unsupported::{current_os_id, set_name};
    }
    target_os = "soso" => {
        mod soso;
        pub use soso::{DEFAULT_MIN_STACK_SIZE, Thread, available_parallelism};
        #[expect(dead_code)]
        mod unsupported;
        pub use unsupported::{current_os_id, set_name, sleep, yield_now};
    }
    _ => {
        mod unsupported;
        pub use unsupported::{
"#),
                    ),
                ],
            },
        },
        Paso {
            nombre: String::from("sys/time: rama soso"),
            fichero: String::from("library/std/src/sys/time/mod.rs"),
            marca: String::from(r#"target_os = "soso" => {"#),
            accion: Accion::Cambios {
                cambios: vec![
                    (
                        String::from(r#"        mod xous;
        use xous as imp;
    }
    _ => {
        mod unsupported;
        use unsupported as imp;
"#),
                        String::from(r#"        mod xous;
        use xous as imp;
    }
    target_os = "soso" => {
        mod soso;
        use soso as imp;
    }
    _ => {
        mod unsupported;
        use unsupported as imp;
"#),
                    ),
                ],
            },
        },
        Paso {
            nombre: String::from("sys/path/unix: cfg soso"),
            fichero: String::from("library/std/src/sys/path/unix.rs"),
            marca: String::from(r#"if cfg!(any(unix, target_os = "hermit", target_os = "wasi", target_os = "motor", target_os = "soso")) {"#),
            accion: Accion::Cambios {
                cambios: vec![
                    (
                        String::from(r#"}

pub(crate) fn is_absolute(path: &Path) -> bool {
    if cfg!(any(unix, target_os = "hermit", target_os = "wasi", target_os = "motor")) {
        path.has_root()
    } else {
        path.has_root() && path.prefix().is_some()
"#),
                        String::from(r#"}

pub(crate) fn is_absolute(path: &Path) -> bool {
    if cfg!(any(unix, target_os = "hermit", target_os = "wasi", target_os = "motor", target_os = "soso")) {
        path.has_root()
    } else {
        path.has_root() && path.prefix().is_some()
"#),
                    ),
                ],
            },
        },
        Paso {
            nombre: String::from("sys/paths: rama soso"),
            fichero: String::from("library/std/src/sys/paths/mod.rs"),
            marca: String::from(r#"target_os = "soso" => {"#),
            accion: Accion::Cambios {
                cambios: vec![
                    (
                        String::from(r#"        mod windows;
        use windows as imp;
    }
    _ => {
        mod unsupported;
        use unsupported as imp;
"#),
                        String::from(r#"        mod windows;
        use windows as imp;
    }
    target_os = "soso" => {
        mod soso;
        #[expect(dead_code)]
        mod unsupported;
        mod imp {
            pub use super::soso::{
                JoinPathsError, SplitPaths, getcwd, join_paths, split_paths,
            };
            pub use super::unsupported::{
                chdir, current_exe, home_dir, temp_dir,
            };
        }
    }
    _ => {
        mod unsupported;
        use unsupported as imp;
"#),
                    ),
                ],
            },
        },
        Paso {
            nombre: String::from("sys/pipe: rama soso"),
            fichero: String::from("library/std/src/sys/pipe/mod.rs"),
            marca: String::from(r#"target_os = "soso" => {"#),
            accion: Accion::Cambios {
                cambios: vec![
                    (
                        String::from(r#"        mod motor;
        pub use motor::{Pipe, pipe};
    }
    _ => {
        mod unsupported;
        pub use unsupported::{Pipe, pipe};
"#),
                        String::from(r#"        mod motor;
        pub use motor::{Pipe, pipe};
    }
    target_os = "soso" => {
        mod soso;
        pub use soso::{Pipe, pipe};
    }
    _ => {
        mod unsupported;
        pub use unsupported::{Pipe, pipe};
"#),
                    ),
                ],
            },
        },
        Paso {
            nombre: String::from("sys/sync/thread_parking: soso"),
            fichero: String::from("library/std/src/sys/sync/thread_parking/mod.rs"),
            marca: String::from(r#"target_os = "soso","#),
            accion: Accion::Cambios {
                cambios: vec![
                    (
                        String::from(r#"        target_os = "fuchsia",
        target_os = "motor",
        target_os = "hermit",
        all(target_os = "wasi", target_env = "p3"),
    ) => {
        mod futex;
"#),
                        String::from(r#"        target_os = "fuchsia",
        target_os = "motor",
        target_os = "hermit",
        target_os = "soso",
        all(target_os = "wasi", target_env = "p3"),
    ) => {
        mod futex;
"#),
                    ),
                ],
            },
        },
        Paso {
            nombre: String::from("sys/process: spawn, wait y salida (C-033..C-111)"),
            fichero: String::from("library/std/src/sys/process/unsupported.rs"),
            marca: String::from(r#"// C-101: extremo de una tubería ya creada (`From<ChildPipe>`)."#),
            accion: Accion::Cambios {
                cambios: vec![
                    (
                        String::from(r#"    Inherit,
    Null,
    MakePipe,
    ParentStdout,
    ParentStderr,
    #[allow(dead_code)] // This variant exists only for the Debug impl
"#),
                        String::from(r#"    Inherit,
    Null,
    MakePipe,
    // C-101: extremo de una tubería ya creada (`From<ChildPipe>`).
    #[allow(dead_code)]
    Fd(u64),
    ParentStdout,
    ParentStderr,
    #[allow(dead_code)] // This variant exists only for the Debug impl
"#),
                    ),
                    (
                        String::from(r#"        _default: Stdio,
        _needs_stdin: bool,
    ) -> io::Result<(Process, StdioPipes)> {
        unsupported()
    }
}

pub fn output(_cmd: &mut Command) -> io::Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    unsupported()
}

impl From<ChildPipe> for Stdio {
    fn from(pipe: ChildPipe) -> Stdio {
        pipe.diverge()
    }
}

"#),
                        String::from(r#"        _default: Stdio,
        _needs_stdin: bool,
    ) -> io::Result<(Process, StdioPipes)> {
        // C-063: soso arranca el programa con SYS_SPAWN_IO. (C-111: con entorno.)
        #[cfg(target_os = "soso")]
        {
            return self.spawn_soso(_default, _needs_stdin);
        }
        #[cfg(not(target_os = "soso"))]
        unsupported()
    }

    #[cfg(target_os = "soso")]
    fn spawn_soso(&mut self, default: Stdio, needs_stdin: bool) -> io::Result<(Process, StdioPipes)> {
        // C-101: stdio por tubería. Los extremos del hijo se cierran en el
        // padre tras el spawn; si no, el lector nunca ve EOF.
        use crate::os::soso::ffi::OsStrExt;
        let mut hijo: Vec<ChildPipe> = Vec::new();
        let mut pipes = StdioPipes { stdin: None, stdout: None, stderr: None };
        let por_defecto_stdin = if needs_stdin { &default } else { &Stdio::Null };
        let stdin = self.stdin.as_ref().unwrap_or(por_defecto_stdin);
        let stdout = self.stdout.as_ref().unwrap_or(&default);
        let stderr = self.stderr.as_ref().unwrap_or(&default);
        let stdin_fd = match stdin {
            Stdio::MakePipe => {
                let (r, w) = crate::sys::pipe::pipe()?;
                let fd = r.fd();
                hijo.push(r);
                pipes.stdin = Some(w);
                fd
            }
            otro => fd_stdio(otro),
        };
        let stdout_fd = match stdout {
            Stdio::MakePipe => {
                let (r, w) = crate::sys::pipe::pipe()?;
                let fd = w.fd();
                hijo.push(w);
                pipes.stdout = Some(r);
                fd
            }
            otro => fd_stdio(otro),
        };
        let stderr_fd = match stderr {
            Stdio::MakePipe => {
                let (r, w) = crate::sys::pipe::pipe()?;
                let fd = w.fd();
                hijo.push(w);
                pipes.stderr = Some(r);
                fd
            }
            otro => fd_stdio(otro),
        };
        let prog = self.program.as_bytes();
        if prog.is_empty() {
            return Err(io::Error::from_raw_os_error(soso_rt::EINVAL as i32));
        }
        let mut argv_table = Vec::with_capacity(self.args.len() * 2);
        for arg in &self.args {
            let bytes = arg.as_bytes();
            argv_table.push(bytes.as_ptr().expose_provenance() as u64);
            argv_table.push(bytes.len() as u64);
        }
        let cwd_bytes: &[u8] = match self.cwd.as_ref() {
            Some(cwd) => cwd.as_bytes(),
            None => b"",
        };
        // C-111: el entorno completo (el actual más los cambios del `Command`);
        // si no hay cambios se hereda el del padre (`envp_ptr == 0`).
        let entorno: Vec<Vec<u8>> = match self.env.capture_if_changed() {
            Some(mapa) => mapa
                .into_iter()
                .map(|(k, v)| {
                    let mut b = k.as_bytes().to_vec();
                    b.push(b'=');
                    b.extend_from_slice(v.as_bytes());
                    b
                })
                .collect(),
            None => Vec::new(),
        };
        let mut envp_tabla = Vec::with_capacity(entorno.len() * 2);
        for e in &entorno {
            envp_tabla.push(e.as_ptr().expose_provenance() as u64);
            envp_tabla.push(e.len() as u64);
        }
        let opts = soso_rt::SpawnIo {
            path_ptr: prog.as_ptr().expose_provenance() as u64,
            path_len: prog.len() as u64,
            args_ptr: 0,
            args_len: 0,
            stdin_fd,
            stdout_fd,
            stderr_fd,
            argv_ptr: if argv_table.is_empty() {
                0
            } else {
                argv_table.as_ptr().expose_provenance() as u64
            },
            argv_count: self.args.len() as u64,
            envp_ptr: if envp_tabla.is_empty() {
                0
            } else {
                envp_tabla.as_ptr().expose_provenance() as u64
            },
            envp_count: entorno.len() as u64,
            log_fd: soso_rt::FD_KERNEL_LOG,
            cwd_ptr: cwd_bytes.as_ptr().expose_provenance() as u64,
            cwd_len: cwd_bytes.len() as u64,
        };
        // SAFETY: `prog`, `argv_table` y `cwd_bytes` viven en esta llamada.
        let rc = unsafe {
            soso_rt::syscall3(
                soso_rt::SYS_SPAWN_IO,
                (&opts as *const soso_rt::SpawnIo).expose_provenance() as u64,
                0,
                0,
            )
        };
        drop(hijo);
        if rc < 0 {
            Err(io::Error::from_raw_os_error((-rc) as i32))
        } else {
            Ok((Process { pid: rc as u64 }, pipes))
        }
    }
}

#[cfg(target_os = "soso")]
fn fd_stdio(s: &Stdio) -> u64 {
    match s {
        Stdio::Null => soso_rt::FD_CLOSED,
        Stdio::Fd(fd) => *fd,
        Stdio::ParentStdout => 1,
        Stdio::ParentStderr => 2,
        _ => soso_rt::FD_INHERIT_TTY,
    }
}

pub fn output(cmd: &mut Command) -> io::Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    // C-101: stdout y stderr se leen a la vez (un hilo) para que un hijo que
    // llene una tubería no se cuelgue esperando a que se vacíe la otra.
    #[cfg(target_os = "soso")]
    {
        let (mut proc, pipes) = cmd.spawn(Stdio::MakePipe, false)?;
        drop(pipes.stdin);
        let mut so = Vec::new();
        let mut se = Vec::new();
        crate::thread::scope(|s| {
            let err = pipes.stderr.map(|p| {
                s.spawn(move || {
                    let mut v = Vec::new();
                    let _ = p.read_to_end(&mut v);
                    v
                })
            });
            if let Some(p) = pipes.stdout {
                let _ = p.read_to_end(&mut so);
            }
            if let Some(h) = err {
                se = h.join().unwrap_or_default();
            }
        });
        let status = proc.wait()?;
        return Ok((status, so, se));
    }
    #[cfg(not(target_os = "soso"))]
    {
        let _ = cmd;
        unsupported()
    }
}

impl From<ChildPipe> for Stdio {
    fn from(pipe: ChildPipe) -> Stdio {
        // C-101: marca de este parche.
        Stdio::Fd(pipe.into_fd())
    }
}

"#),
                    ),
                    (
                        String::from(r#"    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
pub struct ExitStatus();

impl ExitStatus {
    pub fn exit_ok(&self) -> Result<(), ExitStatusError> {
        Ok(())
    }

    pub fn code(&self) -> Option<i32> {
        Some(0)
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<dummy exit status>")
    }
}

pub struct ExitStatusError(!);

impl Clone for ExitStatusError {
    fn clone(&self) -> ExitStatusError {
        self.0
    }
}

impl Copy for ExitStatusError {}

impl PartialEq for ExitStatusError {
    fn eq(&self, _other: &ExitStatusError) -> bool {
        self.0
    }
}

impl Eq for ExitStatusError {}

impl fmt::Debug for ExitStatusError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
"#),
                        String::from(r#"    }
}

#[cfg(target_os = "soso")]
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitStatus(u8);

#[cfg(not(target_os = "soso"))]
#[derive(PartialEq, Eq, Clone, Copy, Debug, Default)]
pub struct ExitStatus();

#[cfg(target_os = "soso")]
impl Default for ExitStatus {
    fn default() -> Self {
        ExitStatus(0)
    }
}

impl ExitStatus {
    pub fn exit_ok(&self) -> Result<(), ExitStatusError> {
        #[cfg(target_os = "soso")]
        {
            return if self.0 == 0 { Ok(()) } else { Err(ExitStatusError(self.0)) };
        }
        #[cfg(not(target_os = "soso"))]
        Ok(())
    }

    pub fn code(&self) -> Option<i32> {
        #[cfg(target_os = "soso")]
        {
            return Some(self.0 as i32);
        }
        #[cfg(not(target_os = "soso"))]
        Some(0)
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(target_os = "soso")]
        {
            return write!(f, "exit status: {}", self.0);
        }
        #[cfg(not(target_os = "soso"))]
        write!(f, "<dummy exit status>")
    }
}

#[cfg(target_os = "soso")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExitStatusError(u8);

#[cfg(not(target_os = "soso"))]
pub struct ExitStatusError(!);

#[cfg(not(target_os = "soso"))]
impl Clone for ExitStatusError {
    fn clone(&self) -> ExitStatusError {
        self.0
    }
}

#[cfg(not(target_os = "soso"))]
impl Copy for ExitStatusError {}

#[cfg(not(target_os = "soso"))]
impl PartialEq for ExitStatusError {
    fn eq(&self, _other: &ExitStatusError) -> bool {
        self.0
    }
}

#[cfg(not(target_os = "soso"))]
impl Eq for ExitStatusError {}

#[cfg(not(target_os = "soso"))]
impl fmt::Debug for ExitStatusError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
"#),
                    ),
                    (
                        String::from(r#"
impl Into<ExitStatus> for ExitStatusError {
    fn into(self) -> ExitStatus {
        self.0
    }
}

impl ExitStatusError {
    pub fn code(self) -> Option<NonZero<i32>> {
        self.0
    }
}
"#),
                        String::from(r#"
impl Into<ExitStatus> for ExitStatusError {
    fn into(self) -> ExitStatus {
        #[cfg(target_os = "soso")]
        {
            return ExitStatus(self.0);
        }
        #[cfg(not(target_os = "soso"))]
        self.0
    }
}

impl ExitStatusError {
    pub fn code(self) -> Option<NonZero<i32>> {
        #[cfg(target_os = "soso")]
        {
            return NonZero::new(self.0 as i32);
        }
        #[cfg(not(target_os = "soso"))]
        self.0
    }
}
"#),
                    ),
                    (
                        String::from(r#"    }
}

pub struct Process(!);

impl Process {
    pub fn id(&self) -> u32 {
        self.0
    }

    pub fn kill(&mut self) -> io::Result<()> {
        self.0
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.0
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0
    }
}
"#),
                        String::from(r#"    }
}

#[cfg(target_os = "soso")]
pub struct Process {
    pid: u64,
}

#[cfg(not(target_os = "soso"))]
pub struct Process(!);

impl Process {
    pub fn id(&self) -> u32 {
        #[cfg(target_os = "soso")]
        {
            return self.pid as u32;
        }
        #[cfg(not(target_os = "soso"))]
        self.0
    }

    pub fn kill(&mut self) -> io::Result<()> {
        #[cfg(target_os = "soso")]
        {
            let _ = self.pid;
            return unsupported();
        }
        #[cfg(not(target_os = "soso"))]
        self.0
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        // C-065: SYS_WAIT espera a cualquier hijo. rustc lanza un enlazador.
        #[cfg(target_os = "soso")]
        {
            let _ = self.pid;
            let rc = unsafe { soso_rt::syscall1(soso_rt::SYS_WAIT, 0) };
            if rc < 0 {
                return Err(io::Error::from_raw_os_error((-rc) as i32));
            }
            let (_hijo, code) = soso_rt::wait_decode(rc);
            return Ok(ExitStatus(code));
        }
        #[cfg(not(target_os = "soso"))]
        self.0
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(target_os = "soso")]
        {
            let _ = self.pid;
            return unsupported();
        }
        #[cfg(not(target_os = "soso"))]
        self.0
    }
}
"#),
                    ),
                    (
                        String::from(r#"    out: ChildPipe,
    _stdout: &mut Vec<u8>,
    _err: ChildPipe,
    _stderr: &mut Vec<u8>,
) -> io::Result<()> {
    match out.diverge() {}
}

pub fn getpid() -> u32 {
    panic!("no pids on this platform")
}
"#),
                        String::from(r#"    out: ChildPipe,
    _stdout: &mut Vec<u8>,
    _err: ChildPipe,
    stderr: &mut Vec<u8>,
) -> io::Result<()> {
    // C-101b: un hilo para stderr, como en `output`.
    #[cfg(target_os = "soso")]
    {
        let mut v = Vec::new();
        crate::thread::scope(|s| {
            let h = s.spawn(move || {
                let mut e = Vec::new();
                let _ = _err.read_to_end(&mut e);
                e
            });
            let _ = out.read_to_end(&mut v);
            *stderr = h.join().unwrap_or_default();
        });
        *_stdout = v;
        return Ok(());
    }
    #[cfg(not(target_os = "soso"))]
    {
        match out.diverge() {}
    }
}

pub fn getpid() -> u32 {
    // C-033: soso tiene SYS_GETPID.
    #[cfg(target_os = "soso")]
    {
        let rc = unsafe { soso_rt::syscall0(soso_rt::SYS_GETPID) };
        return if rc < 0 { 0 } else { rc as u32 };
    }
    #[cfg(not(target_os = "soso"))]
    panic!("no pids on this platform")
}
"#),
                    ),
                ],
            },
        },
    ]
}

use crate::FromEnvErrorInner;
use std::io;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{Builder, JoinHandle};

#[derive(Debug)]
pub struct Client {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    count: Mutex<usize>,
    cvar: Condvar,
}

#[derive(Debug)]
pub struct Acquired(());

impl Client {
    pub fn new(limit: usize) -> io::Result<Client> {
        Ok(Client {
            inner: Arc::new(Inner {
                count: Mutex::new(limit),
                cvar: Condvar::new(),
            }),
        })
    }

    pub(crate) unsafe fn open(_s: &str, _check_pipe: bool) -> Result<Client, FromEnvErrorInner> {
        Err(FromEnvErrorInner::Unsupported)
    }

    pub fn acquire(&self) -> io::Result<Acquired> {
        let mut lock = self.inner.count.lock().unwrap_or_else(|e| e.into_inner());
        while *lock == 0 {
            lock = self
                .inner
                .cvar
                .wait(lock)
                .unwrap_or_else(|e| e.into_inner());
        }
        *lock -= 1;
        Ok(Acquired(()))
    }

    // C-144 jobserver soso: adquisición auxiliar cancelable
    fn acquire_cancelable(&self, cancelled: &AtomicBool) -> io::Result<Option<Acquired>> {
        let mut lock = self.inner.count.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Ok(None);
            }
            if *lock > 0 {
                *lock -= 1;
                return Ok(Some(Acquired(())));
            }
            lock = self
                .inner
                .cvar
                .wait(lock)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    fn cancel_acquire(&self, cancelled: &AtomicBool) {
        // El mismo mutex que protege el predicado de `acquire_cancelable`
        // cierra la ventana comprobar→dormir. `notify_all` es necesario:
        // otro auxiliar puede compartir esta condvar y consumir un notify_one.
        let _lock = self.inner.count.lock().unwrap_or_else(|e| e.into_inner());
        cancelled.store(true, Ordering::Release);
        self.inner.cvar.notify_all();
    }

    pub fn try_acquire(&self) -> io::Result<Option<Acquired>> {
        let mut lock = self.inner.count.lock().unwrap_or_else(|e| e.into_inner());
        if *lock == 0 {
            Ok(None)
        } else {
            *lock -= 1;
            Ok(Some(Acquired(())))
        }
    }

    pub fn release(&self, _data: Option<&Acquired>) -> io::Result<()> {
        let mut lock = self.inner.count.lock().unwrap_or_else(|e| e.into_inner());
        *lock += 1;
        drop(lock);
        self.inner.cvar.notify_one();
        Ok(())
    }

    // C-104 jobserver soso
    pub fn string_arg(&self) -> String {
        String::new()
    }

    pub fn available(&self) -> io::Result<usize> {
        let lock = self.inner.count.lock().unwrap_or_else(|e| e.into_inner());
        Ok(*lock)
    }

    pub fn configure(&self, _cmd: &mut Command) {}
}

#[derive(Debug)]
pub struct Helper {
    thread: JoinHandle<()>,
    client: crate::Client,
    cancelled: Arc<AtomicBool>,
}

pub(crate) fn spawn_helper(
    client: crate::Client,
    state: Arc<super::HelperState>,
    mut f: Box<dyn FnMut(io::Result<crate::Acquired>) + Send>,
) -> io::Result<Helper> {
    let join_client = client.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let thread = Builder::new().spawn(move || {
        state.for_each_request(|_| match client.inner.acquire_cancelable(&worker_cancelled) {
            Ok(Some(data)) => f(Ok(crate::Acquired {
                client: client.inner.clone(),
                data,
                disabled: false,
            })),
            Ok(None) => {}
            Err(error) => f(Err(error)),
        });
    })?;

    Ok(Helper {
        thread,
        client: join_client,
        cancelled,
    })
}

impl Helper {
    pub fn join(self) {
        self.client.inner.cancel_acquire(&self.cancelled);
        drop(self.thread.join());
    }
}

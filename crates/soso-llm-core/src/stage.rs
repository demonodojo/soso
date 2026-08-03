//! Staging asíncrono de shards (double-buffer estilo AirLLM).
//!
//! `kick` encola I/O; `wait` une el trabajo antes del cómputo. Con
//! `StdThreadStager` el prefetch corre en un hilo aparte y solapa con matvec.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StageState {
    Idle,
    InFlight,
    Ready,
}

pub struct StagingSlot {
    pub shards: Vec<String>,
    pub state: StageState,
}

impl StagingSlot {
    pub fn new() -> Self {
        Self {
            shards: Vec::new(),
            state: StageState::Idle,
        }
    }

    pub fn clear(&mut self) {
        self.shards.clear();
        self.state = StageState::Idle;
    }
}

/// Ejecuta prefetch síncrono (mapeo + touch de páginas).
pub trait PrefetchSink {
    fn prefetch_shards_sync(&mut self, shards: &[String]);
}

/// Stager cooperativo: encola en `kick`, materializa en `wait`.
pub struct SyncStager {
    layer: StagingSlot,
    moe: StagingSlot,
}

impl SyncStager {
    pub fn new() -> Self {
        Self {
            layer: StagingSlot::new(),
            moe: StagingSlot::new(),
        }
    }

    pub fn kick_layer(&mut self, shards: &[String]) {
        self.layer.shards.clear();
        self.layer.shards.extend(shards.iter().cloned());
        self.layer.state = StageState::InFlight;
    }

    pub fn kick_moe(&mut self, shards: &[String]) {
        self.moe.shards.clear();
        self.moe.shards.extend(shards.iter().cloned());
        self.moe.state = StageState::InFlight;
    }

    pub fn wait_layer<S: PrefetchSink>(&mut self, sink: &mut S) {
        if self.layer.state == StageState::InFlight {
            if !self.layer.shards.is_empty() {
                sink.prefetch_shards_sync(&self.layer.shards);
            }
            self.layer.state = StageState::Ready;
        }
    }

    pub fn wait_moe<S: PrefetchSink>(&mut self, sink: &mut S) {
        if self.moe.state == StageState::InFlight {
            if !self.moe.shards.is_empty() {
                sink.prefetch_shards_sync(&self.moe.shards);
            }
            self.moe.state = StageState::Ready;
        }
    }

    pub fn drain_layer_shards(&mut self) -> Vec<String> {
        if self.layer.state == StageState::InFlight {
            self.layer.state = StageState::Ready;
            core::mem::take(&mut self.layer.shards)
        } else {
            Vec::new()
        }
    }

    pub fn drain_moe_shards(&mut self) -> Vec<String> {
        if self.moe.state == StageState::InFlight {
            self.moe.state = StageState::Ready;
            core::mem::take(&mut self.moe.shards)
        } else {
            Vec::new()
        }
    }

    pub fn cancel(&mut self) {
        self.layer.clear();
        self.moe.clear();
    }

    pub fn layer_in_flight(&self) -> bool {
        self.layer.state == StageState::InFlight
    }

    pub fn moe_in_flight(&self) -> bool {
        self.moe.state == StageState::InFlight
    }

    pub fn layer_shards(&self) -> &[String] {
        &self.layer.shards
    }

    pub fn moe_shards(&self) -> &[String] {
        &self.moe.shards
    }
}

/// Stager con hilo de prefetch (host `std` o userspace con callback externo).
pub struct AsyncStager {
    inner: SyncStager,
    worker_busy: core::sync::atomic::AtomicU32,
}

impl AsyncStager {
    pub fn new() -> Self {
        Self {
            inner: SyncStager::new(),
            worker_busy: core::sync::atomic::AtomicU32::new(0),
        }
    }

    pub fn kick_layer(&mut self, shards: &[String]) {
        self.inner.kick_layer(shards);
    }

    pub fn kick_moe(&mut self, shards: &[String]) {
        self.inner.kick_moe(shards);
    }

    pub fn wait_layer<S: PrefetchSink>(&mut self, sink: &mut S) {
        // Si un worker externo marcó busy=0, el prefetch ya corrió.
        if self.worker_busy.load(core::sync::atomic::Ordering::Acquire) == 0 {
            self.inner.wait_layer(sink);
        } else {
            // Worker aún activo: esperar spin breve o caer a sync.
            while self.worker_busy.load(core::sync::atomic::Ordering::Acquire) != 0 {
                core::hint::spin_loop();
            }
            self.inner.layer.state = StageState::Ready;
        }
    }

    pub fn wait_moe<S: PrefetchSink>(&mut self, sink: &mut S) {
        self.inner.wait_moe(sink);
    }

    pub fn cancel(&mut self) {
        self.inner.cancel();
        self.worker_busy
            .store(0, core::sync::atomic::Ordering::Release);
    }

    pub fn mark_worker_busy(&self) {
        self.worker_busy
            .store(1, core::sync::atomic::Ordering::Release);
    }

    pub fn mark_worker_done(&self) {
        self.worker_busy
            .store(0, core::sync::atomic::Ordering::Release);
    }

    pub fn inner_mut(&mut self) -> &mut SyncStager {
        &mut self.inner
    }

    pub fn inner(&self) -> &SyncStager {
        &self.inner
    }
}

#[cfg(feature = "std")]
pub mod host {
    use super::*;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};

    enum Job {
        Layer(Vec<String>),
        Moe(Vec<String>),
        Shutdown,
    }

    enum Done {
        Layer,
        Moe,
    }

    /// Prefetch en hilo std (AirLLM ThreadPoolExecutor).
    pub struct StdThreadStager<S: PrefetchSink + Send + 'static> {
        stager: AsyncStager,
        tx: Sender<Job>,
        rx_done: Receiver<Done>,
        _handle: JoinHandle<()>,
        _marker: core::marker::PhantomData<S>,
    }

    impl<S: PrefetchSink + Send + 'static> StdThreadStager<S> {
        pub fn new(sink: Arc<Mutex<S>>) -> Self {
            let (tx, rx) = mpsc::channel();
            let (tx_done, rx_done) = mpsc::channel();
            let stager = AsyncStager::new();
            let handle = thread::spawn(move || {
                while let Ok(job) = rx.recv() {
                    match job {
                        Job::Layer(shards) => {
                            if let Ok(mut s) = sink.lock() {
                                s.prefetch_shards_sync(&shards);
                            }
                            let _ = tx_done.send(Done::Layer);
                        }
                        Job::Moe(shards) => {
                            if let Ok(mut s) = sink.lock() {
                                s.prefetch_shards_sync(&shards);
                            }
                            let _ = tx_done.send(Done::Moe);
                        }
                        Job::Shutdown => break,
                    }
                }
            });
            Self {
                stager,
                tx,
                rx_done,
                _handle: handle,
                _marker: core::marker::PhantomData,
            }
        }

        pub fn kick_layer(&mut self, shards: &[String]) {
            self.wait_layer();
            self.stager.kick_layer(shards);
            let _ = self.tx.send(Job::Layer(shards.to_vec()));
        }

        pub fn kick_moe(&mut self, shards: &[String]) {
            self.wait_moe();
            self.stager.kick_moe(shards);
            let _ = self.tx.send(Job::Moe(shards.to_vec()));
        }

        pub fn wait_layer(&mut self) {
            while self.stager.inner().layer_in_flight() {
                match self.rx_done.recv() {
                    Ok(Done::Layer) => {
                        self.stager.inner_mut().layer.state = StageState::Ready;
                        break;
                    }
                    Ok(Done::Moe) => {}
                    Err(_) => break,
                }
            }
        }

        pub fn wait_moe(&mut self) {
            while self.stager.inner().moe_in_flight() {
                match self.rx_done.recv() {
                    Ok(Done::Moe) => {
                        self.stager.inner_mut().moe.state = StageState::Ready;
                        break;
                    }
                    Ok(Done::Layer) => {}
                    Err(_) => break,
                }
            }
        }
    }

    impl<S: PrefetchSink + Send + 'static> Drop for StdThreadStager<S> {
        fn drop(&mut self) {
            let _ = self.tx.send(Job::Shutdown);
        }
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    struct CountingSink {
        pub calls: Arc<AtomicU32>,
        pub delay_ms: u64,
    }

    impl PrefetchSink for CountingSink {
        fn prefetch_shards_sync(&mut self, _shards: &[String]) {
            if self.delay_ms > 0 {
                thread::sleep(Duration::from_millis(self.delay_ms));
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn sync_stager_kick_wait_defers_io() {
        let mut stager = SyncStager::new();
        let shards = vec![String::from("a.tensor")];
        stager.kick_layer(&shards);
        assert!(stager.layer_in_flight());
        let mut sink = CountingSink {
            calls: Arc::new(AtomicU32::new(0)),
            delay_ms: 0,
        };
        assert_eq!(sink.calls.load(Ordering::Relaxed), 0);
        stager.wait_layer(&mut sink);
        assert_eq!(sink.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn std_thread_stager_overlaps_with_compute() {
        let calls = Arc::new(AtomicU32::new(0));
        let sink = Arc::new(Mutex::new(CountingSink {
            calls: calls.clone(),
            delay_ms: 50,
        }));
        let mut stager = host::StdThreadStager::new(sink);
        let shards = vec![String::from("L00.attn_q.tensor")];
        stager.kick_layer(&shards);
        let t0 = std::time::Instant::now();
        // Simular cómputo mientras el worker prefetchea.
        thread::sleep(Duration::from_millis(10));
        let compute_ms = t0.elapsed().as_millis();
        stager.wait_layer();
        let total_ms = t0.elapsed().as_millis();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        // Solape: total < compute + 50ms de prefetch puro.
        assert!(
            total_ms < compute_ms + 45,
            "total={total_ms} compute={compute_ms}"
        );
    }
}

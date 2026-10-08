use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(Instant::now() < deadline, "no se alcanzó el estado esperado");
        thread::yield_now();
    }
}

fn drop_async(helper: HelperThread) -> (thread::JoinHandle<()>, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        drop(helper);
        tx.send(()).unwrap();
    });
    (handle, rx)
}

#[test]
fn pending_acquisition_is_cancelled() {
    let client = Client::new(1).unwrap();
    client.acquire_raw().unwrap(); // permiso implícito de cargo
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let helper = client
        .clone()
        .into_helper_thread(move |_| {
            callback_calls.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    helper.request_token();
    until(|| helper.state.lock().requests == 0);
    let state = helper.state.clone();

    let (dropper, done) = drop_async(helper);
    done.recv_timeout(Duration::from_secs(2))
        .expect("el cierre no canceló acquire");
    dropper.join().unwrap();

    assert!(state.lock().consumer_done);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.available().unwrap(), 0);
    client.release_raw().unwrap();
    assert_eq!(client.available().unwrap(), 1);
}

#[test]
fn shutdown_without_request_finishes() {
    let client = Client::new(0).unwrap();
    let helper = client.into_helper_thread(|_| panic!("callback inesperado")).unwrap();
    let (dropper, done) = drop_async(helper);
    done.recv_timeout(Duration::from_secs(2)).unwrap();
    dropper.join().unwrap();
}

#[test]
fn cancelling_one_helper_does_not_cancel_another() {
    let client = Client::new(1).unwrap();
    client.acquire_raw().unwrap();
    let calls_one = Arc::new(AtomicUsize::new(0));
    let calls_one_cb = calls_one.clone();
    let one = client
        .clone()
        .into_helper_thread(move |_| {
            calls_one_cb.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let two = client
        .clone()
        .into_helper_thread(move |token| {
            drop(token.unwrap());
            tx.send(()).unwrap();
        })
        .unwrap();
    one.request_token();
    two.request_token();
    until(|| one.state.lock().requests == 0 && two.state.lock().requests == 0);

    let (dropper, done) = drop_async(one);
    done.recv_timeout(Duration::from_secs(2)).unwrap();
    dropper.join().unwrap();
    assert_eq!(calls_one.load(Ordering::SeqCst), 0);

    client.release_raw().unwrap();
    rx.recv_timeout(Duration::from_secs(2))
        .expect("cancelar el primer auxiliar canceló también el segundo");
    drop(two);
    assert_eq!(client.available().unwrap(), 1);
}

#[test]
fn release_racing_with_cancel_preserves_tokens() {
    for _ in 0..64 {
        let client = Client::new(1).unwrap();
        client.acquire_raw().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = calls.clone();
        let helper = client
            .clone()
            .into_helper_thread(move |token| {
                drop(token.unwrap());
                callback_calls.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
        helper.request_token();
        until(|| helper.state.lock().requests == 0);

        let gate = Arc::new(Barrier::new(2));
        let release_gate = gate.clone();
        let release_client = client.clone();
        let releaser = thread::spawn(move || {
            release_gate.wait();
            release_client.release_raw().unwrap();
        });
        gate.wait();
        drop(helper);
        releaser.join().unwrap();

        assert!(calls.load(Ordering::SeqCst) <= 1);
        assert_eq!(client.available().unwrap(), 1);
    }
}

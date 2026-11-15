#![cfg(feature = "std")]

use soso_llm_core::{cluster::kv_transfer::{enviar, recibir}, pipeline::host::TcpTransport};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

#[test]
fn transfiere_una_imagen_entre_dos_extremos_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let contenido: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    let enviado = contenido.clone();
    let hijo = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        enviar(&mut TcpTransport::from_stream(stream), &enviado, 1_000_000).unwrap();
    });
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let recibido = recibir(&mut TcpTransport::from_stream(stream), 1_000_000).unwrap();
    hijo.join().unwrap();
    assert_eq!(recibido, contenido);
}

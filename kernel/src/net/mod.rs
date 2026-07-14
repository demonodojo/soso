//! Pila de red: smoltcp sobre virtio-net. IP estática 10.0.2.15/24 (la
//! red de usuario de QEMU), pasarela 10.0.2.2.
//!
//! Sin interrupciones de red: `poll()` se llama desde el bucle del
//! scheduler, el de la kernel-shell y el tick de timer cuando interrumpe
//! a un proceso de usuario (~10 ms de latencia máxima). Usa try_lock:
//! si la pila está ocupada, la próxima pasada lo recoge.

mod device;
pub mod ssh;

use crate::arch::pit;
use crate::drivers::virtio_net;
use crate::println;
use alloc::vec;
use alloc::vec::Vec;
use device::SmolDev;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use spin::{Mutex, Once};

pub const ECHO_PORT: u16 = 7;

/// Sockets de eco escuchando a la vez: mientras uno atiende o se cierra,
/// otro sigue en LISTEN (si no, una segunda conexión rápida recibe RST).
const ECHO_SOCKETS: usize = 4;

struct NetStack {
    iface: Interface,
    sockets: SocketSet<'static>,
    echo: Vec<SocketHandle>,
    ssh: SocketHandle,
    dev: SmolDev,
}

static NET: Once<Mutex<NetStack>> = Once::new();

fn now() -> Instant {
    Instant::from_millis(pit::uptime_ms() as i64)
}

pub fn init() {
    let Some(mac) = virtio_net::init() else { return };

    let mut dev = SmolDev;
    let config = Config::new(EthernetAddress(mac).into());
    let mut iface = Interface::new(config, &mut dev, now());
    iface.update_ip_addrs(|addrs| {
        addrs.push(IpCidr::new(IpAddress::v4(10, 0, 2, 15), 24)).unwrap();
    });
    iface
        .routes_mut()
        .add_default_ipv4_route(Ipv4Address::new(10, 0, 2, 2))
        .unwrap();

    let mut sockets = SocketSet::new(Vec::new());
    let echo = (0..ECHO_SOCKETS)
        .map(|_| {
            sockets.add(tcp::Socket::new(
                tcp::SocketBuffer::new(vec![0; 8192]),
                tcp::SocketBuffer::new(vec![0; 8192]),
            ))
        })
        .collect();
    // Socket del servidor SSH (puerto 22): buffers grandes para paquetes
    // SSH y ráfagas de salida de la shell.
    let ssh = sockets.add(tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; 16384]),
        tcp::SocketBuffer::new(vec![0; 16384]),
    ));

    println!(
        "net: 10.0.2.15/24 (mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}), echo en :{ECHO_PORT}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    ssh::init();
    NET.call_once(|| Mutex::new(NetStack { iface, sockets, echo, ssh, dev }));
}

/// Procesa la red: entrada/salida pendiente y el servidor de eco.
/// Reentrante-seguro vía try_lock (puede llamarse desde el tick de timer
/// que interrumpió a un proceso de usuario: ahí el kernel no tiene locks).
pub fn poll() {
    let Some(net) = NET.get() else { return };
    let Some(mut n) = net.try_lock() else { return };
    let NetStack { iface, sockets, echo, ssh, dev } = &mut *n;

    iface.poll(now(), dev, sockets);

    // Servidor SSH (puerto 22).
    ssh::poll(sockets.get_mut::<tcp::Socket>(*ssh));

    for &h in echo.iter() {
        let s = sockets.get_mut::<tcp::Socket>(h);
        if !s.is_open() {
            s.listen(ECHO_PORT).ok();
            continue;
        }
        // Eco: reenviar lo recibido mientras quepa en el buffer de salida.
        let mut buf = [0u8; 1024];
        while s.can_recv() && s.can_send() {
            let leidos = s.recv_slice(&mut buf).unwrap_or(0);
            if leidos == 0 {
                break;
            }
            s.send_slice(&buf[..leidos]).ok();
        }
        // El cliente cerró su lado: cerrar el nuestro al vaciarse todo.
        if s.state() == tcp::State::CloseWait && !s.can_recv() {
            s.close();
        }
    }

    iface.poll(now(), dev, sockets);
}

//! Pila de red: smoltcp sobre virtio-net. DHCP al arrancar; si no hay lease
//! en unos segundos, fallback a IP estática 10.0.2.15/24 (QEMU slirp).
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
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{EthernetAddress, IpCidr, Ipv4Address, Ipv4Cidr};
use spin::{Mutex, Once};

pub const ECHO_PORT: u16 = 7;

/// Sockets de eco escuchando a la vez: mientras uno atiende o se cierra,
/// otro sigue en LISTEN (si no, una segunda conexión rápida recibe RST).
const ECHO_SOCKETS: usize = 4;

/// Tiempo máximo de espera de DHCP antes del fallback estático.
const DHCP_TIMEOUT: Duration = Duration::from_secs(8);

/// IP y pasarela del fallback QEMU slirp.
const FALLBACK_ADDR: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
const FALLBACK_GW: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

struct NetStack {
    iface: Interface,
    sockets: SocketSet<'static>,
    echo: Vec<SocketHandle>,
    ssh: SocketHandle,
    dhcp: SocketHandle,
    configured: bool,
    dhcp_started: Instant,
    dev: SmolDev,
}

static NET: Once<Mutex<NetStack>> = Once::new();

fn now() -> Instant {
    Instant::from_millis(pit::uptime_ms() as i64)
}

fn random_seed() -> u64 {
    let mut buf = [0u8; 8];
    if getrandom::getrandom(&mut buf).is_ok() {
        u64::from_le_bytes(buf)
    } else {
        pit::uptime_ms() as u64
    }
}

fn set_ipv4_addr(iface: &mut Interface, cidr: Ipv4Cidr) {
    iface.update_ip_addrs(|addrs| {
        addrs.clear();
        addrs.push(IpCidr::Ipv4(cidr)).unwrap();
    });
}

fn apply_static_fallback(iface: &mut Interface) {
    set_ipv4_addr(iface, Ipv4Cidr::new(FALLBACK_ADDR, 24));
    iface.routes_mut().remove_default_ipv4_route();
    iface
        .routes_mut()
        .add_default_ipv4_route(FALLBACK_GW)
        .unwrap();
}

fn clear_ipv4_config(iface: &mut Interface) {
    iface.update_ip_addrs(|addrs| addrs.clear());
    iface.routes_mut().remove_default_ipv4_route();
}

fn close_tcp_services(sockets: &mut SocketSet<'static>, echo: &[SocketHandle], ssh: SocketHandle) {
    for &h in echo {
        let s = sockets.get_mut::<tcp::Socket>(h);
        if s.is_open() {
            s.close();
        }
    }
    let s = sockets.get_mut::<tcp::Socket>(ssh);
    if s.is_open() {
        s.close();
    }
}

pub fn init() {
    let Some(mac) = virtio_net::init() else { return };

    let mut dev = SmolDev;
    let mut config = Config::new(EthernetAddress(mac).into());
    config.random_seed = random_seed();
    let iface = Interface::new(config, &mut dev, now());

    let mut sockets = SocketSet::new(Vec::new());
    let dhcp = sockets.add(dhcpv4::Socket::new());
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
        "net: dhcp… (mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    ssh::init();
    let dhcp_started = now();
    NET.call_once(|| {
        Mutex::new(NetStack {
            iface,
            sockets,
            echo,
            ssh,
            dhcp,
            configured: false,
            dhcp_started,
            dev,
        })
    });
}

fn poll_dhcp(
    iface: &mut Interface,
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: SocketHandle,
    dhcp: SocketHandle,
    configured: &mut bool,
) {
    let event = sockets.get_mut::<dhcpv4::Socket>(dhcp).poll();
    match event {
        None => {}
        Some(dhcpv4::Event::Configured(config)) => {
            set_ipv4_addr(iface, config.address);
            iface.routes_mut().remove_default_ipv4_route();
            if let Some(router) = config.router {
                iface.routes_mut().add_default_ipv4_route(router).unwrap();
                println!(
                    "net: dhcp {}/{} gw {}",
                    config.address.address(),
                    config.address.prefix_len(),
                    router
                );
            } else {
                println!(
                    "net: dhcp {}/{}",
                    config.address.address(),
                    config.address.prefix_len()
                );
            }
            *configured = true;
        }
        Some(dhcpv4::Event::Deconfigured) => {
            // smoltcp emite Deconfigured al arrancar (estado Discovering con
            // config_changed=true); ignorar si aún no hubo lease.
            if !*configured {
                return;
            }
            println!("net: dhcp perdido");
            clear_ipv4_config(iface);
            close_tcp_services(sockets, echo, ssh);
            *configured = false;
        }
    }
}

fn try_static_fallback(iface: &mut Interface, dhcp_started: Instant, configured: &mut bool) {
    if *configured || now() < dhcp_started + DHCP_TIMEOUT {
        return;
    }
    apply_static_fallback(iface);
    *configured = true;
    println!("net: sin dhcp, ip estática {FALLBACK_ADDR}/24");
}

fn poll_tcp_services(
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: SocketHandle,
    configured: bool,
) {
    if !configured {
        return;
    }

    // Servidor SSH (puerto 22).
    ssh::poll(sockets.get_mut::<tcp::Socket>(ssh));

    for &h in echo {
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
}

/// Procesa la red: DHCP, entrada/salida pendiente y el servidor de eco.
/// Reentrante-seguro vía try_lock (puede llamarse desde el tick de timer
/// que interrumpió a un proceso de usuario: ahí el kernel no tiene locks).
pub fn poll() {
    let Some(net) = NET.get() else { return };
    let Some(mut n) = net.try_lock() else { return };
    let NetStack {
        iface,
        sockets,
        echo,
        ssh,
        dhcp,
        configured,
        dhcp_started,
        dev,
    } = &mut *n;

    iface.poll(now(), dev, sockets);
    poll_dhcp(iface, sockets, echo, *ssh, *dhcp, configured);
    try_static_fallback(iface, *dhcp_started, configured);
    poll_tcp_services(sockets, echo, *ssh, *configured);
    iface.poll(now(), dev, sockets);
}

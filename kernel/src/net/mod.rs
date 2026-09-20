//! Pila de red: smoltcp sobre e1000e, virtio-net, rtl8169 o WiFi Intel (AX211/AX200).
//! DHCP al arrancar en backends cableados; en WiFi sólo tras asociación.
//! Fallback 10.0.2.x únicamente en QEMU (virtio/e1000e).

mod device;
pub mod dns;
mod loopback;
mod ping;
pub mod ssh;
mod tcp_user;
#[cfg(feature = "lxdde")]
pub mod wifi_wpa;

use crate::arch::pit;
use crate::println;
use alloc::vec;
use alloc::vec::Vec;
use device::NicDev;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address, Ipv4Cidr};
use spin::{Mutex, Once};

#[cfg(feature = "drv-e1000e")]
use crate::drivers::e1000e;
#[cfg(feature = "drv-e1000e")]
use device::E1000Dev;
#[cfg(feature = "drv-rtl8169")]
use crate::drivers::rtl8169;
#[cfg(feature = "drv-virtio-net")]
use crate::drivers::virtio_net;

pub const ECHO_PORT: u16 = 7;
const ECHO_SOCKETS: usize = 4;
const DHCP_TIMEOUT: Duration = Duration::from_secs(8);
const FALLBACK_GW: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    Wired,
    Wifi,
    /// Sin NIC: la pila existe sólo para el loopback de usuario.
    Ninguno,
}

/// MAC administrada localmente para la pila sin NIC. Nunca sale a un cable.
const MAC_SIN_NIC: [u8; 6] = [0x02, 0x50, 0x53, 0x4f, 0x53, 0x4f];

struct NetStack {
    iface: Interface,
    sockets: SocketSet<'static>,
    echo: Vec<SocketHandle>,
    ssh: Vec<SocketHandle>,
    dhcp: SocketHandle,
    configured: bool,
    dhcp_enabled: bool,
    dhcp_started: Instant,
    dev: NicDev,
    mac: [u8; 6],
    backend: BackendKind,
    /// Servidores DNS que anunció el DHCP. Sin esto había que adivinarlos, y
    /// lo que se adivinaba era el de QEMU.
    ///
    /// Array fijo, **no** un `Vec`: esto se rellena desde `poll_dhcp`, que
    /// corre también desde el tick del temporizador —o sea, desde una
    /// interrupción—. Reservar memoria ahí dentro puede pillar al asignador a
    /// medias y dejar su lista de bloques corrupta; se paga mucho después, con
    /// un page fault dentro de `talc` que no se parece en nada a su causa.
    dns: [Option<smoltcp::wire::Ipv4Address>; 3],
    user_tcp: tcp_user::TcpTable,
}

static NET: Once<Mutex<NetStack>> = Once::new();
static LOCK_PERDIDO: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn now() -> Instant {
    Instant::from_millis(pit::uptime_ms() as i64)
}

fn net_backend() -> Option<([u8; 6], NicDev, BackendKind)> {
    #[cfg(feature = "lxdde")]
    if crate::lxdde::wifi_present()
        && crate::lxdde::wifi_alive()
        && crate::lxdde::wifi_connected()
    {
        if let Some(mac) = crate::lxdde::wifi_mac() {
            println!("net: backend lx-wifi (Intel AX211/AX200)");
            return Some((mac, NicDev::LxWifi(device::LxWifiDev), BackendKind::Wifi));
        }
    }
    #[cfg(feature = "lxdde")]
    if crate::lxdde::e1000e_present() {
        let mac = crate::lxdde::e1000e_mac()?;
        println!("net: backend lx-e1000e");
        return Some((mac, NicDev::LxE1000e(device::LxE1000Dev), BackendKind::Wired));
    }
    #[cfg(feature = "drv-e1000e")]
    if e1000e::present() {
        let mac = e1000e::mac()?;
        println!("net: backend e1000e");
        return Some((mac, NicDev::E1000e(E1000Dev), BackendKind::Wired));
    }
    #[cfg(feature = "drv-rtl8169")]
    if rtl8169::present() {
        let mac = rtl8169::mac()?;
        println!("net: backend rtl8169");
        return Some((mac, NicDev::Rtl8169(device::Rtl8169Dev), BackendKind::Wired));
    }
    #[cfg(feature = "lxdde")]
    if crate::lxdde::wifi_present() && crate::lxdde::wifi_alive() {
        if let Some(mac) = crate::lxdde::wifi_mac() {
            println!("net: backend lx-wifi (Intel AX211/AX200)");
            return Some((mac, NicDev::LxWifi(device::LxWifiDev), BackendKind::Wifi));
        }
    }
    #[cfg(feature = "drv-virtio-net")]
    if let Some(mac) = virtio_net::init() {
        println!("net: backend virtio-net");
        return Some((mac, NicDev::Virtio(device::SmolDev), BackendKind::Wired));
    }
    None
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

fn apply_static_fallback(iface: &mut Interface, mac: [u8; 6]) {
    let addr = tcp_user::fallback_addr_from_mac(mac);
    set_ipv4_addr(iface, Ipv4Cidr::new(addr, 24));
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

fn close_tcp_services(
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: &[SocketHandle],
) {
    for &h in echo {
        let s = sockets.get_mut::<tcp::Socket>(h);
        if s.is_open() {
            s.close();
        }
    }
    for &h in ssh {
        let s = sockets.get_mut::<tcp::Socket>(h);
        if s.is_open() {
            s.close();
        }
    }
}

fn attach_stack(mac: [u8; 6], mut dev: NicDev, backend: BackendKind, dhcp_now: bool) {
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
    let ssh = (0..ssh::SSH_SESSIONS)
        .map(|_| {
            sockets.add(tcp::Socket::new(
                tcp::SocketBuffer::new(vec![0; 16384]),
                tcp::SocketBuffer::new(vec![0; 16384]),
            ))
        })
        .collect();

    if backend == BackendKind::Ninguno {
        println!("net: sin NIC — pila sólo para loopback (127.0.0.1)");
    } else {
        println!(
            "net: dhcp… (mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
    }
    ssh::init();
    let dhcp_started = if dhcp_now { now() } else { Instant::from_millis(0) };
    NET.call_once(|| {
        Mutex::new(NetStack {
            dns: [None; 3],
            iface,
            sockets,
            echo,
            ssh,
            dhcp,
            configured: false,
            dhcp_enabled: dhcp_now,
            dhcp_started,
            dev,
            mac,
            backend,
            user_tcp: tcp_user::TcpTable::new(),
        })
    });
}

/// Reintento de sondeo mientras no hay pila: `poll()` entra aquí en cada vuelta
/// del bucle ocioso y del tick, y sondear el bus a esa cadencia no aporta nada.
const RESONDEO_MS: u64 = 1000;
static ULTIMO_SONDEO: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// ¿Hay ya una NIC de verdad detrás de la pila? (la pila de loopback no cuenta)
static NIC_REAL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Intenta enganchar una NIC de verdad (WiFi ALIVE incluido).
/// Limitada en frecuencia: para forzar el intento, `attach_now()`.
pub fn try_attach() {
    if NIC_REAL.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let ahora = pit::uptime_ms();
    let ultimo = ULTIMO_SONDEO.load(core::sync::atomic::Ordering::Relaxed);
    if ultimo != 0 && ahora.saturating_sub(ultimo) < RESONDEO_MS {
        return;
    }
    ULTIMO_SONDEO.store(ahora.max(1), core::sync::atomic::Ordering::Relaxed);
    attach_now();
}

/// Sondeo inmediato, sin esperar al reintento (arranque y asociación WiFi).
///
/// Si no hay NIC monta igualmente la pila con backend `Ninguno`: el TCP de
/// usuario vive dentro de `NetStack`, así que sin ella `tcp_listen` falla y el
/// loopback —o sea, `ask`— no existe. Si más tarde aparece una NIC de verdad
/// (el WiFi que asocia tarde), se sustituye el dispositivo **en sitio**, sin
/// tirar los sockets: el askd que ya estaba escuchando sigue escuchando.
pub fn attach_now() {
    if NIC_REAL.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some((mac, dev, backend)) = net_backend() else {
        if NET.get().is_none() {
            attach_stack(MAC_SIN_NIC, NicDev::Ninguno, BackendKind::Ninguno, false);
        }
        return;
    };
    let dhcp_now = backend == BackendKind::Wired
        || (backend == BackendKind::Wifi && wifi_link_up());
    NIC_REAL.store(true, core::sync::atomic::Ordering::Relaxed);
    match NET.get() {
        None => attach_stack(mac, dev, backend, dhcp_now),
        Some(net) => sustituir_nic(net, mac, dev, backend, dhcp_now),
    }
}

/// Cambia el dispositivo de una pila ya montada (era `Ninguno`) por una NIC real.
fn sustituir_nic(
    net: &Mutex<NetStack>,
    mac: [u8; 6],
    dev: NicDev,
    backend: BackendKind,
    dhcp_now: bool,
) {
    let mut n = net.lock();
    n.dev = dev;
    n.mac = mac;
    n.backend = backend;
    n.iface
        .set_hardware_addr(EthernetAddress(mac).into());
    n.configured = false;
    n.dhcp_enabled = dhcp_now;
    n.dhcp_started = now();
    clear_ipv4_config(&mut n.iface);
    let dhcp = n.dhcp;
    n.sockets.get_mut::<dhcpv4::Socket>(dhcp).reset();
    println!(
        "net: NIC encontrada tras arrancar sin ella (mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
}

#[cfg(feature = "lxdde")]
fn wifi_link_up() -> bool {
    // Autorizada, no sólo asociada: con WPA2 a medias el AP tira todo lo que
    // salga y DHCP se queda reintentando contra un enlace que no transporta.
    crate::lxdde::wifi_authorized()
}

#[cfg(not(feature = "lxdde"))]
fn wifi_link_up() -> bool {
    false
}

/// Tras subir el enlace Ethernet: reinicia DHCP.
#[cfg(feature = "drv-rtl8169")]
pub fn on_wired_link_up() {
    let Some(net) = NET.get() else {
        return;
    };
    let mut n = net.lock();
    if n.backend != BackendKind::Wired {
        return;
    }
    n.configured = false;
    n.dhcp_enabled = true;
    n.dhcp_started = now();
    clear_ipv4_config(&mut n.iface);
    let echo = n.echo.clone();
    let ssh = n.ssh.clone();
    let dhcp = n.dhcp;
    close_tcp_services(&mut n.sockets, &echo, &ssh);
    n.sockets.get_mut::<dhcpv4::Socket>(dhcp).reset();
    println!("net: enlace ethernet UP — solicitando DHCP…");
}

pub fn init() {
    attach_now();
}

/// Tras asociar WiFi: sustituye ethernet por LxWifi si hace falta y pide DHCP.
#[cfg(feature = "lxdde")]
pub fn on_wifi_connected() {
    let Some(net) = NET.get() else {
        attach_now();
        return;
    };
    let mut n = net.lock();
    if crate::lxdde::wifi_authorized() {
        let Some(mac) = crate::lxdde::wifi_mac() else {
            return;
        };
        if n.backend != BackendKind::Wifi || n.mac != mac {
            n.dev = NicDev::LxWifi(device::LxWifiDev);
            n.mac = mac;
            n.backend = BackendKind::Wifi;
            n.iface
                .set_hardware_addr(EthernetAddress(mac).into());
            n.configured = false;
            clear_ipv4_config(&mut n.iface);
            NIC_REAL.store(true, core::sync::atomic::Ordering::Relaxed);
            println!("net: backend lx-wifi (Intel AX211/AX200)");
        }
    } else if n.backend != BackendKind::Wifi {
        drop(n);
        attach_now();
        let Some(net) = NET.get() else {
            return;
        };
        n = net.lock();
        if n.backend != BackendKind::Wifi {
            return;
        }
    }
    n.configured = false;
    n.dhcp_enabled = true;
    n.dhcp_started = now();
    clear_ipv4_config(&mut n.iface);
    let echo = n.echo.clone();
    let ssh = n.ssh.clone();
    let dhcp = n.dhcp;
    close_tcp_services(&mut n.sockets, &echo, &ssh);
    n.sockets.get_mut::<dhcpv4::Socket>(dhcp).reset();
    println!("net: wifi asociada — solicitando DHCP…");
}

fn poll_dhcp(
    iface: &mut Interface,
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: &[SocketHandle],
    dhcp: SocketHandle,
    configured: &mut bool,
    dns: &mut [Option<smoltcp::wire::Ipv4Address>; 3],
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
            if !config.dns_servers.is_empty() {
                *dns = [None; 3];
                for (hueco, servidor) in dns.iter_mut().zip(config.dns_servers.iter()) {
                    *hueco = Some(*servidor);
                    println!("net: dns {servidor}");
                }
            }
            *configured = true;
        }
        Some(dhcpv4::Event::Deconfigured) => {
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

fn try_static_fallback(
    iface: &mut Interface,
    mac: [u8; 6],
    backend: BackendKind,
    dev: &NicDev,
    dhcp_started: Instant,
    configured: &mut bool,
) {
    if backend == BackendKind::Wifi || !dev.fallback_slirp() {
        return;
    }
    if *configured || now() < dhcp_started + DHCP_TIMEOUT {
        return;
    }
    apply_static_fallback(iface, mac);
    *configured = true;
    let addr = tcp_user::fallback_addr_from_mac(mac);
    println!("net: sin dhcp, ip estática {addr}/24");
}

fn poll_tcp_services(
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: &[SocketHandle],
    configured: bool,
) {
    if !configured {
        return;
    }
    for (slot, &h) in ssh.iter().enumerate() {
        ssh::poll(slot, sockets.get_mut::<tcp::Socket>(h));
    }
    for &h in echo {
        let s = sockets.get_mut::<tcp::Socket>(h);
        if !s.is_open() {
            s.listen(ECHO_PORT).ok();
            continue;
        }
        let mut buf = [0u8; 1024];
        while s.can_recv() && s.can_send() {
            let leidos = s.recv_slice(&mut buf).unwrap_or(0);
            if leidos == 0 {
                break;
            }
            s.send_slice(&buf[..leidos]).ok();
        }
        if s.state() == tcp::State::CloseWait && !s.can_recv() {
            s.close();
        }
    }
}

static TRABAJO_PENDIENTE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

pub fn marcar_trabajo_pendiente() {
    TRABAJO_PENDIENTE.store(true, core::sync::atomic::Ordering::Release);
}

pub fn trabajo_pendiente() -> bool {
    TRABAJO_PENDIENTE.swap(false, core::sync::atomic::Ordering::AcqRel)
}

pub fn poll() {
    debug_assert!(
        !crate::arch::irq::en_irq_dura(),
        "net::poll() desde IRQ dura: reentraría en PROCS/RX/TX/heap"
    );
    try_attach();
    #[cfg(feature = "drv-rtl8169")]
    if rtl8169::poll_link() {
        on_wired_link_up();
    }
    #[cfg(feature = "drv-e1000e")]
    if e1000e::poll_link() {
        on_wired_link_up();
    }
    let Some(net) = NET.get() else { return };
    let Some(mut n) = net.try_lock() else {
        // Quien se lleva el candado y no lo suelta deja la pila sin sondear: ni
        // avanza una conexión ni vencen los plazos. Se cuenta de vez en cuando
        // para no ahogar la consola.
        let fallos = LOCK_PERDIDO.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if fallos.is_multiple_of(1000) {
            crate::println!("net: poll sin candado ({fallos} veces)");
        }
        return;
    };
    TRABAJO_PENDIENTE.store(false, core::sync::atomic::Ordering::Relaxed);
    let NetStack {
        iface,
        sockets,
        echo,
        ssh,
        dhcp,
        configured,
        dhcp_enabled,
        dhcp_started,
        dev,
        mac,
        backend,
        dns,
        user_tcp,
    } = &mut *n;

    iface.poll(now(), dev, sockets);
    if *dhcp_enabled {
        poll_dhcp(iface, sockets, echo, ssh, *dhcp, configured, dns);
        try_static_fallback(iface, *mac, *backend, dev, *dhcp_started, configured);
    }
    poll_tcp_services(sockets, echo, ssh, *configured);
    poll_user_tcp(iface, sockets, user_tcp, *configured);
    iface.poll(now(), dev, sockets);
}

fn poll_user_tcp(
    iface: &mut Interface,
    sockets: &mut SocketSet<'static>,
    user_tcp: &mut tcp_user::TcpTable,
    configured: bool,
) {
    for slot in 0..user_tcp.entries.len() {
        if let Some(entry) = user_tcp.entries[slot].as_mut() {
            tcp_user::poll_entry(iface, sockets, slot, entry, configured);
        }
    }
}

pub fn tcp_listen(port: u16) -> Result<usize, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let NetStack {
        sockets,
        user_tcp,
        ..
    } = &mut *n;
    if user_tcp.find_listener(port).is_some() {
        return Err(-soso_abi::EADDRINUSE);
    }
    let slot = user_tcp
        .alloc(sockets, tcp_user::TcpRole::Listening, port, None)
        .map_err(|_| -soso_abi::EMFILE)?;
    if let Some(entry) = user_tcp.entries[slot].as_mut() {
        entry.loop_listener = true;
        // El accept de userspace es loopback (127.0.0.1); smoltcp listen es
        // opcional. Sin IP (NicDev::Ninguno) puede fallar, y si abortáramos
        // aquí el askd no levantaba nunca en placa sin driver de red.
        let _ = tcp_user::listen_start(sockets, entry);
    }
    Ok(slot)
}

pub fn tcp_connect(remote: soso_abi::SockAddr) -> Result<usize, i64> {
    if loopback::is_loopback_addr(&remote.addr) {
        return tcp_connect_loopback(remote.port);
    }
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let NetStack {
        sockets,
        user_tcp,
        ..
    } = &mut *n;
    let ep = tcp_user::endpoint_from_abi(&remote);
    let slot = user_tcp
        .alloc(
            sockets,
            tcp_user::TcpRole::Connecting,
            0,
            Some(ep),
        )
        .map_err(|_| -soso_abi::EMFILE)?;
    Ok(slot)
}

fn tcp_connect_loopback(port: u16) -> Result<usize, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let listener_slot = n
        .user_tcp
        .find_listener(port)
        .ok_or(-soso_abi::ECONNREFUSED)?;
    let pair_id = loopback::alloc_pair();
    let client_slot = {
        let NetStack { sockets, user_tcp, .. } = &mut *n;
        user_tcp
            .alloc_loopback(
                sockets,
                tcp_user::TcpRole::Connected,
                pair_id,
                loopback::LoopSide::Client,
            )
            .map_err(|_| -soso_abi::EMFILE)?
    };
    let server_slot = {
        let NetStack { sockets, user_tcp, .. } = &mut *n;
        user_tcp
            .alloc_loopback(
                sockets,
                tcp_user::TcpRole::Connected,
                pair_id,
                loopback::LoopSide::Server,
            )
            .map_err(|_| -soso_abi::EMFILE)?
    };
    loopback::register_pending(listener_slot, server_slot);
    Ok(client_slot)
}

/// Acepta una conexión loopback pendiente. Devuelve el slot del lado servidor.
pub fn tcp_accept_loopback(listener_slot: usize) -> Result<usize, i64> {
    loopback::take_pending(listener_slot).ok_or(-soso_abi::EAGAIN)
}

pub fn tcp_accept(listener_slot: usize) -> Result<usize, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let n = net.lock();
    let entry = n
        .user_tcp
        .entries
        .get(listener_slot)
        .and_then(|e| e.as_ref())
        .ok_or(-soso_abi::EBADF)?;
    if entry.loop_listener {
        drop(n);
        return tcp_accept_loopback(listener_slot);
    }
    if entry.role != tcp_user::TcpRole::Listening && entry.role != tcp_user::TcpRole::Connected {
        return Err(-soso_abi::EINVAL);
    }
    if tcp_user::is_established(&n.sockets, entry.handle) {
        drop(n);
        let net = NET.get().unwrap();
        let mut n = net.lock();
        if let Some(e) = n
            .user_tcp
            .entries
            .get_mut(listener_slot)
            .and_then(|x| x.as_mut())
        {
            e.role = tcp_user::TcpRole::Connected;
        }
        return Ok(listener_slot);
    }
    Err(-soso_abi::EAGAIN)
}

pub fn tcp_try_read(slot: usize, buf: u64, len: u64) -> Result<u64, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    // Ver la nota de `tcp_is_connected`. «Cero bytes» es una respuesta que el
    // llamante ya sabe tratar: vuelve a dormirse y se reintenta.
    let Some(mut n) = net.try_lock() else { return Ok(0) };
    let entry = n
        .user_tcp
        .entries
        .get(slot)
        .and_then(|e| e.as_ref())
        .ok_or(-soso_abi::EBADF)?;
    if entry.closed {
        return Ok(0);
    }
    if entry.loopback {
        return tcp_user::try_read_loopback(entry, buf, len);
    }
    let handle = entry.handle;
    let role = entry.role;
    tcp_user::try_read(&mut n.sockets, handle, role, buf, len)
}

pub fn tcp_try_write(slot: usize, buf: u64, len: u64) -> Result<u64, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    // Ver la nota de `tcp_is_connected`.
    let Some(mut n) = net.try_lock() else { return Ok(0) };
    let entry = n
        .user_tcp
        .entries
        .get(slot)
        .and_then(|e| e.as_ref())
        .ok_or(-soso_abi::EBADF)?;
    if entry.closed {
        return Err(-soso_abi::EPIPE);
    }
    if entry.loopback {
        return tcp_user::try_write_loopback(entry, buf, len);
    }
    let handle = entry.handle;
    let role = entry.role;
    tcp_user::try_write(&mut n.sockets, handle, role, buf, len)
}

pub fn tcp_close(slot: usize) {
    if let Some(net) = NET.get() {
        let mut n = net.lock();
        let NetStack {
            sockets,
            user_tcp,
            ..
        } = &mut *n;
        user_tcp.free(sockets, slot);
    }
}

/// **Nunca `lock()` bloqueante aquí.** Estas funciones las llama el planificador
/// con el candado de **procesos** tomado, mientras el servicio SSH —que corre
/// bajo el candado de **red**— llama a `task::exists`, `spawn_console` y
/// compañía, que toman el de procesos. Los dos órdenes juntos son un abrazo
/// mortal con dos cores: uno tiene red y quiere procesos, el otro al revés.
/// Nadie sale, la pila deja de sondearse y ningún plazo vence.
///
/// Contestar «todavía no» cuando el candado está ocupado es seguro: quien
/// pregunta vuelve a intentarlo en la vuelta siguiente del planificador.
/// Lo que se sabe de un socket después de intentar mirar la red.
///
/// `Ocupado` **no es un estado del socket**: es «la red estaba tomada y no se ha
/// podido mirar». Tiene que ir aparte porque no todas las preguntas admiten la
/// misma respuesta por defecto. Quien espera a que una conexión se establezca
/// puede tratar «no lo sé» como «todavía no» sin perder nada; quien va a
/// devolverle **EOF** a un proceso, no: eso es una respuesta definitiva y
/// equivocarse cuesta la conexión entera.
///
/// Con `tcp_is_connected` devolviendo `false` en ese caso —como hizo entre el
/// arreglo del abrazo mortal y el 2026-09-20—, la primera lectura después del
/// ClientHello volvía con **0 bytes en 0 ms**, `soso-http` lo leía como «el par
/// dejó de mandar» y el handshake TLS moría siempre contra un servidor real. En
/// bucle local no salía: el eco contesta antes de que haga falta preguntar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EstadoTcp {
    Conectado,
    Cerrado,
    Ocupado,
}

pub fn tcp_estado(slot: usize) -> EstadoTcp {
    let Some(net) = NET.get() else {
        return EstadoTcp::Cerrado;
    };
    let Some(n) = net.try_lock() else {
        return EstadoTcp::Ocupado;
    };
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return EstadoTcp::Cerrado;
    };
    if entry.role != tcp_user::TcpRole::Connected || entry.closed {
        return EstadoTcp::Cerrado;
    }
    if entry.loopback {
        let Some(pair) = entry.loop_pair else {
            return EstadoTcp::Cerrado;
        };
        // Sin esto, el cierre del par (askd muerto) dejaba al cliente
        // «conectado» para siempre: `try_read` devolvía 0, `tcp_is_connected`
        // seguía en true y `read_timeout` bloqueaba otra vez → EAGAIN eterno
        // (sosh giraba en `copiar_respuesta_ask`, 2026-08-31).
        if loopback::peer_closed(pair, entry.loop_side) && !loopback::has_unread(pair, entry.loop_side)
        {
            return EstadoTcp::Cerrado;
        }
    }
    EstadoTcp::Conectado
}

/// `true` sólo si **consta** que está conectado. Un `false` puede ser «no lo sé».
pub fn tcp_is_connected(slot: usize) -> bool {
    tcp_estado(slot) == EstadoTcp::Conectado
}

/// `true` sólo si **consta** que está cerrado. Es la pregunta que hay que hacer
/// antes de devolverle EOF a un proceso; `!tcp_is_connected` no vale.
pub fn tcp_cerrado_seguro(slot: usize) -> bool {
    tcp_estado(slot) == EstadoTcp::Cerrado
}

#[allow(dead_code)]
pub fn tcp_is_connecting(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    let n = net.lock();
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    entry.role == tcp_user::TcpRole::Connecting && !entry.closed
}

pub fn tcp_connect_failed(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    // Ver la nota de `tcp_is_connected`: aquí tampoco se espera por la red.
    let Some(n) = net.try_lock() else { return false };
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    entry.role == tcp_user::TcpRole::Connecting && entry.closed
}

pub fn tcp_slot_loop_listener(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    let n = net.lock();
    n.user_tcp
        .entries
        .get(slot)
        .and_then(|e| e.as_ref())
        .is_some_and(|e| e.loop_listener)
}

/// Resultado de accept al despertar un waiter: `None` = mismo fd; `Some(slot)` = fd nuevo.
pub fn tcp_accept_wake(listener_slot: usize) -> Result<Option<usize>, i64> {
    if !tcp_listener_ready(listener_slot) {
        return Err(-soso_abi::EAGAIN);
    }
    if tcp_slot_loop_listener(listener_slot) {
        return tcp_accept_loopback(listener_slot).map(Some);
    }
    tcp_accept(listener_slot)?;
    Ok(None)
}

pub use ping::ping;

/// Foto de la IPv4 de la NIC activa. La pila sin adaptador no cuenta como presente.
pub fn info() -> soso_abi::NetInfo {
    let mut out = soso_abi::NetInfo::default();
    let Some(net) = NET.get() else {
        return out;
    };
    let n = net.lock();
    out.mac = n.mac;
    out.backend = match n.backend {
        BackendKind::Ninguno => soso_abi::NET_BACKEND_NONE,
        BackendKind::Wired => soso_abi::NET_BACKEND_WIRED,
        BackendKind::Wifi => soso_abi::NET_BACKEND_WIFI,
    };
    if n.backend != BackendKind::Ninguno {
        out.flags |= soso_abi::NET_FLAG_PRESENT;
    }
    if n.configured {
        out.flags |= soso_abi::NET_FLAG_CONFIGURED;
    }
    if let Some(IpCidr::Ipv4(v4)) = n.iface.ip_addrs().first().copied() {
        out.addr = v4.address().octets();
        out.prefix_len = v4.prefix_len();
    }
    if let Some(route) = n.iface.routes().get_default_ipv4_route() {
        let IpAddress::Ipv4(gw) = route.via_router;
        out.gateway = gw.octets();
    }
    out
}

/// Texto del comando `ip` (kshell). El binario de userspace imprime lo mismo.
pub fn print_info() {
    let info = info();
    if info.flags & soso_abi::NET_FLAG_PRESENT == 0 {
        println!("ip: sin adaptador de red");
        return;
    }
    let medio = match info.backend {
        soso_abi::NET_BACKEND_WIFI => "wifi",
        _ => "ethernet",
    };
    if info.flags & soso_abi::NET_FLAG_CONFIGURED == 0 {
        println!("ip: sin dirección (esperando DHCP)");
    } else if info.gateway != [0, 0, 0, 0] {
        println!(
            "ip: {}.{}.{}.{}/{} gw {}.{}.{}.{}",
            info.addr[0],
            info.addr[1],
            info.addr[2],
            info.addr[3],
            info.prefix_len,
            info.gateway[0],
            info.gateway[1],
            info.gateway[2],
            info.gateway[3]
        );
    } else {
        println!(
            "ip: {}.{}.{}.{}/{}",
            info.addr[0], info.addr[1], info.addr[2], info.addr[3], info.prefix_len
        );
    }
    println!(
        "  mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  {medio}",
        info.mac[0], info.mac[1], info.mac[2], info.mac[3], info.mac[4], info.mac[5]
    );
}

pub fn tcp_listener_ready(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    // Ver la nota de `tcp_is_connected`.
    let Some(n) = net.try_lock() else { return false };
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    if entry.loop_listener {
        return loopback::has_pending(slot);
    }
    tcp_user::is_established(&n.sockets, entry.handle)
}

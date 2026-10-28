//! Pila de red: smoltcp sobre e1000e, virtio-net, rtl8169 o WiFi Intel (AX211/AX200).
//! DHCP al arrancar en backends cableados; en WiFi sólo tras asociación.
//! Fallback 10.0.2.x únicamente en QEMU (virtio/e1000e).

pub(crate) mod device;
pub mod dns;
mod loopback;
mod ping;
pub mod ssh;
mod tcp_user;
#[cfg(feature = "lxdde")]
pub mod wifi_wpa;

use crate::arch::pit;
use crate::{klogln, println};
use alloc::vec;
use alloc::vec::Vec;
use device::NicDev;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{DhcpRepr, EthernetAddress, IpAddress, IpCidr, Ipv4Address, Ipv4Cidr};
use soso_red_core::RedEstatica;
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
    /// `None` sólo con dirección fija: un socket DHCP vivo seguiría mandando
    /// DISCOVER (ver [`parar_dhcp`]).
    dhcp: Option<SocketHandle>,
    configured: bool,
    dhcp_enabled: bool,
    dhcp_started: Instant,
    dev: NicDev,
    mac: [u8; 6],
    backend: BackendKind,
    /// Dirección de `/etc/red.conf`. `None` es DHCP.
    estatica: Option<RedEstatica>,
    /// El usuario eligió WiFi con `wifi connect`: el cable con enlace no la
    /// sustituye hasta `ip medio auto` o hasta que la WiFi caiga.
    pref_wifi: bool,
    /// Servidores DNS que anunció el DHCP. Sin esto había que adivinarlos, y
    /// lo que se adivinaba era el de QEMU.
    ///
    /// Array fijo, **no** un `Vec`: esto se rellena desde `poll_dhcp`, que
    /// corre también desde el tick del temporizador —o sea, desde una
    /// interrupción—. Reservar memoria ahí dentro puede pillar al asignador a
    /// medias y dejar su lista de bloques corrupta; se paga mucho después, con
    /// un page fault dentro de `talc` que no se parece en nada a su causa.
    dns: [Option<smoltcp::wire::Ipv4Address>; 3],
    /// Fin de lease según la última concesión DHCP (opción 51).
    dhcp_expires_at: Option<Instant>,
    /// Dirección QEMU 10.0.2.x por timeout DHCP; no confundir con un lease real.
    respaldo: bool,
    user_tcp: tcp_user::TcpTable,
}

static mut DHCP_RX_BUF: [u8; 1500] = [0u8; 1500];

fn dhcp_rx_slice() -> &'static mut [u8] {
    // SAFETY: un solo socket DHCP usa este búfer; `preparar_socket_dhcp` no
    // mantiene referencias cruzadas entre sockets distintos.
    unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(DHCP_RX_BUF).cast(), 1500)
    }
}

fn preparar_socket_dhcp(s: &mut dhcpv4::Socket) {
    s.set_receive_packet_buffer(dhcp_rx_slice());
}

fn nuevo_socket_dhcp() -> dhcpv4::Socket<'static> {
    let mut s = dhcpv4::Socket::new();
    preparar_socket_dhcp(&mut s);
    s
}

static NET: Once<Mutex<NetStack>> = Once::new();
static LOCK_PERDIDO: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// `/etc/red.conf` tal como estaba al arrancar. Se lee una vez en [`init`]: la
/// pila se monta (y se rehace al aparecer una NIC) desde `poll`, que corre en
/// el tick y no toca ficheros.
static ESTATICA_ARRANQUE: Once<Option<RedEstatica>> = Once::new();

/// Lee `/etc/red.conf`. Ausente o `modo=dhcp` es DHCP; un fichero que no vale
/// se cuenta y se trata como DHCP, para no dejar la máquina sin red por un
/// error de tecleo.
fn leer_red_conf() -> Option<RedEstatica> {
    let ino = crate::vfs::resolve(soso_red_core::RED_CONF).ok()?;
    let datos = crate::vfs::read_file(ino).ok()?;
    let texto = core::str::from_utf8(&datos).unwrap_or("");
    match soso_red_core::parsear_red_conf(texto) {
        Ok(estatica) => estatica,
        Err(e) => {
            klogln!("net: {} no vale ({}); uso DHCP", soso_red_core::RED_CONF, e.texto());
            None
        }
    }
}

fn guardar_red_conf(texto: &str) -> bool {
    if crate::fs::FS.get().is_none() {
        return false;
    }
    let Ok(etc) = crate::vfs::resolve("/etc") else {
        return false;
    };
    crate::vfs::create_file(etc, "red.conf", texto.as_bytes(), crate::time::wall_secs()).is_ok()
}

fn now() -> Instant {
    Instant::from_millis(pit::uptime_ms() as i64)
}

/// Ethernet nativa o lxdde presente, con o sin enlace, y el nombre para el log.
/// Sin virtio: ése sólo existe en QEMU, donde no compite con ningún WiFi.
/// Es muda; quien elige la NIC es quien lo cuenta.
fn sondear_cable_nativo() -> Option<([u8; 6], NicDev, &'static str)> {
    #[cfg(feature = "lxdde")]
    if crate::lxdde::e1000e_present() {
        let mac = crate::lxdde::e1000e_mac()?;
        return Some((mac, NicDev::LxE1000e(device::LxE1000Dev), "lx-e1000e"));
    }
    #[cfg(feature = "drv-e1000e")]
    if e1000e::present() {
        let mac = e1000e::mac()?;
        return Some((mac, NicDev::E1000e(E1000Dev), "e1000e"));
    }
    #[cfg(feature = "drv-rtl8169")]
    if rtl8169::present() {
        let mac = rtl8169::mac()?;
        return Some((mac, NicDev::Rtl8169(device::Rtl8169Dev), "rtl8169"));
    }
    None
}

/// MAC de la WiFi viva; con `asociada`, sólo si además está asociada.
#[cfg(feature = "lxdde")]
fn sondear_wifi(asociada: bool) -> Option<[u8; 6]> {
    if crate::lxdde::wifi_present()
        && crate::lxdde::wifi_alive()
        && (!asociada || crate::lxdde::wifi_connected())
    {
        crate::lxdde::wifi_mac()
    } else {
        None
    }
}

#[cfg(not(feature = "lxdde"))]
fn sondear_wifi(_asociada: bool) -> Option<[u8; 6]> {
    None
}

fn backend_wifi(mac: [u8; 6]) -> Option<([u8; 6], NicDev, BackendKind)> {
    #[cfg(feature = "lxdde")]
    {
        klogln!("net: backend lx-wifi (Intel AX211/AX200)");
        Some((mac, NicDev::LxWifi(device::LxWifiDev), BackendKind::Wifi))
    }
    #[cfg(not(feature = "lxdde"))]
    {
        let _ = mac;
        None
    }
}

/// Orden: cable con enlace, WiFi asociada, cable sin enlace (el DHCP arranca
/// al subirlo), WiFi viva y virtio. El cable con enlace gana a una WiFi que
/// ya está asociada; la WiFi sólo gana por elección explícita
/// (`NetStack::pref_wifi`), que no existe todavía cuando se monta la pila.
fn net_backend() -> Option<([u8; 6], NicDev, BackendKind)> {
    let cable = sondear_cable_nativo();
    let cable_con_enlace = cable.as_ref().is_some_and(|(_, dev, _)| dev.carrier());
    if !cable_con_enlace {
        if let Some(mac) = sondear_wifi(true) {
            return backend_wifi(mac);
        }
    }
    if let Some((mac, dev, nombre)) = cable {
        klogln!("net: backend {nombre}");
        return Some((mac, dev, BackendKind::Wired));
    }
    if let Some(mac) = sondear_wifi(false) {
        return backend_wifi(mac);
    }
    #[cfg(feature = "drv-virtio-net")]
    if let Some(mac) = virtio_net::init() {
        klogln!("net: backend virtio-net");
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
    let dhcp = Some(sockets.add(nuevo_socket_dhcp()));
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
        klogln!(
            "net: dhcp… (mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
    }
    ssh::init();
    let estatica = ESTATICA_ARRANQUE.get().copied().flatten();
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
            estatica,
            pref_wifi: false,
            dhcp_expires_at: None,
            respaldo: false,
            user_tcp: tcp_user::TcpTable::new(),
        })
    });
    // Sólo con enlace usable: una WiFi aún sin autorizar la recibirá en
    // `on_wifi_connected`, y sin NIC no hay a qué ponérsela.
    if let (Some(e), true, Some(net)) = (estatica, dhcp_now && backend != BackendKind::Ninguno, NET.get()) {
        aplicar_estatica(&mut net.lock(), e);
    }
}

fn dhcp_lease_expires(config: &dhcpv4::Config<'_>, now: Instant) -> Option<Instant> {
    let packet = config.packet.as_ref()?;
    let repr = DhcpRepr::parse(packet).ok()?;
    let secs = repr.lease_duration? as u64;
    Some(now + Duration::from_secs(secs))
}

fn dhcp_deconfig_motivo(now: Instant, expires_at: Option<Instant>) -> &'static str {
    match expires_at {
        Some(exp) if now >= exp => "lease expirado",
        Some(exp) if now + Duration::from_secs(30) >= exp => {
            "renovación fallida (cerca del fin de lease)"
        }
        Some(_) => "NAK o retirada del servidor",
        None => "configuración retirada",
    }
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
    cambiar_dispositivo(&mut n, mac, dev, backend);
    n.configured = false;
    clear_ipv4_config(&mut n.iface);
    match n.estatica {
        Some(e) if dhcp_now => aplicar_estatica(&mut n, e),
        _ => {
            arrancar_dhcp(&mut n);
            n.dhcp_enabled = dhcp_now;
        }
    }
    klogln!(
        "net: NIC encontrada tras arrancar sin ella (mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x})",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
}

/// Pone otro dispositivo bajo la pila. La IP no se toca: quien lo llama la
/// reinicia con [`reiniciar_ip`] (o la suelta con [`soltar_ip`]).
fn cambiar_dispositivo(n: &mut NetStack, mac: [u8; 6], dev: NicDev, backend: BackendKind) {
    n.dev = dev;
    n.mac = mac;
    n.backend = backend;
    n.iface.set_hardware_addr(EthernetAddress(mac).into());
    NIC_REAL.store(true, core::sync::atomic::Ordering::Relaxed);
}

/// Empieza (o reempieza) la búsqueda de lease. El socket DHCP vuelve a existir
/// si una dirección fija lo había retirado.
fn arrancar_dhcp(n: &mut NetStack) {
    match n.dhcp {
        Some(h) => {
            let s = n.sockets.get_mut::<dhcpv4::Socket>(h);
            s.reset();
            preparar_socket_dhcp(s);
        }
        None => n.dhcp = Some(n.sockets.add(nuevo_socket_dhcp())),
    }
    n.dhcp_enabled = true;
    n.dhcp_started = now();
    n.respaldo = false;
}

/// Con dirección fija nadie debe pedir lease: un socket DHCP vivo seguiría
/// mandando DISCOVER a la red de quien eligió no usarlo.
fn parar_dhcp(n: &mut NetStack) {
    if let Some(h) = n.dhcp.take() {
        n.sockets.remove(h);
    }
    n.dhcp_enabled = false;
    n.dhcp_expires_at = None;
}

fn aplicar_estatica(n: &mut NetStack, e: RedEstatica) {
    parar_dhcp(n);
    let addr = Ipv4Address::from(e.addr);
    set_ipv4_addr(&mut n.iface, Ipv4Cidr::new(addr, e.prefijo));
    n.iface.routes_mut().remove_default_ipv4_route();
    if let Some(gw) = e.gw {
        let _ = n.iface.routes_mut().add_default_ipv4_route(Ipv4Address::from(gw));
    }
    n.dns = [None; 3];
    n.dns[0] = e.dns.map(Ipv4Address::from);
    n.configured = true;
    println!("net: ip estática {addr}/{}", e.prefijo);
}

/// La IP de ahora ya no vale (otro medio, enlace nuevo): se suelta, se cierran
/// los servicios que escuchaban en ella y se empieza de nuevo, con la dirección
/// fija si la hay y con DHCP si no.
fn reiniciar_ip(n: &mut NetStack) {
    n.configured = false;
    n.dhcp_expires_at = None;
    n.respaldo = false;
    clear_ipv4_config(&mut n.iface);
    close_tcp_services(&mut n.sockets, &n.echo, &n.ssh);
    match n.estatica {
        Some(e) => aplicar_estatica(n, e),
        None => arrancar_dhcp(n),
    }
}

/// Sin enlace no hay IP: ni dirección ni DHCP hasta que vuelva.
fn soltar_ip(n: &mut NetStack) {
    n.configured = false;
    n.dhcp_enabled = false;
    n.dhcp_expires_at = None;
    n.respaldo = false;
    clear_ipv4_config(&mut n.iface);
    close_tcp_services(&mut n.sockets, &n.echo, &n.ssh);
    if let Some(h) = n.dhcp {
        n.sockets.get_mut::<dhcpv4::Socket>(h).reset();
    }
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

/// MAC de la WiFi si tiene enlace utilizable.
#[cfg(feature = "lxdde")]
fn wifi_autorizada() -> Option<[u8; 6]> {
    if crate::lxdde::wifi_authorized() {
        crate::lxdde::wifi_mac()
    } else {
        None
    }
}

/// Tras subir el enlace Ethernet: reinicia la IP. Si la pila estaba en WiFi por
/// elección de nadie (preferencia `auto`, sin dirección fija), el cable con
/// enlace la recupera.
#[cfg(any(feature = "drv-rtl8169", feature = "drv-e1000e"))]
pub fn on_wired_link_up() {
    let Some(net) = NET.get() else {
        return;
    };
    let mut n = net.lock();
    if n.backend == BackendKind::Wired {
        reiniciar_ip(&mut n);
        if n.estatica.is_some() {
            klogln!("net: enlace ethernet UP — ip estática");
        } else {
            klogln!("net: enlace ethernet UP — solicitando DHCP…");
        }
        return;
    }
    if n.backend != BackendKind::Wifi || n.pref_wifi || n.estatica.is_some() {
        return;
    }
    let Some((mac, dev, nombre)) = sondear_cable_nativo() else {
        return;
    };
    if !dev.carrier() {
        return;
    }
    cambiar_dispositivo(&mut n, mac, dev, BackendKind::Wired);
    reiniciar_ip(&mut n);
    klogln!("net: cable con enlace — vuelvo a ethernet ({nombre})");
}

/// Se soltó el cable: sin IP hasta que vuelva, igual que cuando cae el WiFi.
/// Con dirección fija se conserva (lo que se eligió no depende del cable) y
/// una WiFi ya autorizada recoge la red.
#[cfg(any(feature = "drv-rtl8169", feature = "drv-e1000e"))]
pub fn on_wired_link_down() {
    let Some(net) = NET.get() else {
        return;
    };
    let mut n = net.lock();
    if n.backend != BackendKind::Wired {
        return;
    }
    if n.estatica.is_some() {
        klogln!("net: enlace ethernet DOWN — conservo la ip estática");
        return;
    }
    soltar_ip(&mut n);
    println!("net: enlace ethernet DOWN — sin IP hasta que vuelva el cable");
    #[cfg(feature = "lxdde")]
    if let Some(mac) = wifi_autorizada() {
        cambiar_dispositivo(&mut n, mac, NicDev::LxWifi(device::LxWifiDev), BackendKind::Wifi);
        reiniciar_ip(&mut n);
        klogln!("net: sin cable — uso la WiFi autorizada");
    }
}

pub fn init() {
    // Una sola vez y aquí: `poll` y `try_attach` corren en el tick y no leen
    // ficheros. El sistema de ficheros ya está montado en este punto del arranque.
    ESTATICA_ARRANQUE.call_once(leer_red_conf);
    attach_now();
}

/// Tras asociar WiFi: sustituye ethernet por LxWifi si hace falta y pide DHCP.
/// Con el cable con enlace y sin elección del usuario, sigue en ethernet.
#[cfg(feature = "lxdde")]
pub fn on_wifi_connected() {
    wifi_conectada(false);
}

/// `wifi connect` / `wifi setup`: el usuario pidió esta red, así que la WiFi
/// manda aunque el cable tenga enlace, hasta `ip medio auto` o hasta que caiga.
#[cfg(feature = "lxdde")]
pub fn on_wifi_connected_explicito() {
    wifi_conectada(true);
}

#[cfg(feature = "lxdde")]
fn wifi_conectada(explicito: bool) {
    let Some(net) = NET.get() else {
        attach_now();
        return;
    };
    let mut n = net.lock();
    if explicito {
        n.pref_wifi = true;
    }
    if crate::lxdde::wifi_authorized() {
        let Some(mac) = crate::lxdde::wifi_mac() else {
            return;
        };
        if n.backend == BackendKind::Wired && !n.pref_wifi && n.dev.carrier() {
            klogln!("net: wifi autorizada; sigo en ethernet (el cable tiene enlace)");
            return;
        }
        if n.backend != BackendKind::Wifi || n.mac != mac {
            cambiar_dispositivo(&mut n, mac, NicDev::LxWifi(device::LxWifiDev), BackendKind::Wifi);
            klogln!("net: backend lx-wifi (Intel AX211/AX200)");
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
    reiniciar_ip(&mut n);
    if n.estatica.is_some() {
        klogln!("net: wifi asociada — ip estática");
    } else {
        klogln!("net: wifi asociada — solicitando DHCP…");
    }
}

/// El enlace WiFi cayó: sin IP ni DHCP hasta que `wifi_wpa` reconecte y llame
/// a [`on_wifi_connected`]; lo mismo que un `dhcp perdido`. Si el cable tiene
/// enlace, la pila vuelve a él y la preferencia por WiFi se suelta: el
/// reintento de `wifi_wpa` no debe quitarle otra vez la sesión. Con dirección
/// fija se conserva. Devuelve `false` si el candado estaba tomado (DNS lo
/// retiene mientras espera): se reintenta.
#[cfg(feature = "lxdde")]
pub fn on_wifi_lost() -> bool {
    let Some(net) = NET.get() else {
        return true;
    };
    let Some(mut n) = net.try_lock() else {
        return false;
    };
    if n.backend != BackendKind::Wifi {
        return true;
    }
    if n.estatica.is_some() {
        klogln!("net: wifi caída — conservo la ip estática");
        return true;
    }
    soltar_ip(&mut n);
    println!("net: wifi caída — sin IP hasta reconectar");
    if let Some((mac, dev, nombre)) = sondear_cable_nativo()
        && dev.carrier()
    {
        n.pref_wifi = false;
        cambiar_dispositivo(&mut n, mac, dev, BackendKind::Wired);
        reiniciar_ip(&mut n);
        klogln!("net: cable con enlace — vuelvo a ethernet ({nombre})");
    }
    true
}

fn ipv4_en_iface(iface: &Interface) -> Option<Ipv4Cidr> {
    iface
        .ip_addrs()
        .first()
        .and_then(|c| match c {
            IpCidr::Ipv4(v) => Some(*v),
        })
}

fn dns_cambiaron(
    actual: &[Option<smoltcp::wire::Ipv4Address>; 3],
    nuevos: &[Ipv4Address],
) -> bool {
    let mut vistos = [None; 3];
    for (i, s) in nuevos.iter().take(3).enumerate() {
        vistos[i] = Some(*s);
    }
    vistos != *actual
}

fn poll_dhcp(
    iface: &mut Interface,
    sockets: &mut SocketSet<'static>,
    echo: &[SocketHandle],
    ssh: &[SocketHandle],
    dhcp: SocketHandle,
    configured: &mut bool,
    dns: &mut [Option<smoltcp::wire::Ipv4Address>; 3],
    dhcp_expires_at: &mut Option<Instant>,
    respaldo: &mut bool,
) {
    let event = sockets.get_mut::<dhcpv4::Socket>(dhcp).poll();
    match event {
        None => {}
        Some(dhcpv4::Event::Configured(config)) => {
            *dhcp_expires_at = dhcp_lease_expires(&config, now());
            *respaldo = false;
            let prev = ipv4_en_iface(iface);
            let cambio_ip = prev != Some(config.address) || !*configured;
            if cambio_ip {
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
            }
            if !config.dns_servers.is_empty() && dns_cambiaron(dns, &config.dns_servers) {
                *dns = [None; 3];
                for (hueco, servidor) in dns.iter_mut().zip(config.dns_servers.iter()) {
                    *hueco = Some(*servidor);
                    klogln!("net: dns {servidor}");
                }
            }
            *configured = true;
        }
        Some(dhcpv4::Event::Deconfigured) => {
            if !*configured {
                return;
            }
            let motivo = dhcp_deconfig_motivo(now(), *dhcp_expires_at);
            println!("net: dhcp perdido ({motivo})");
            *dhcp_expires_at = None;
            *respaldo = false;
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
    respaldo: &mut bool,
) {
    if backend == BackendKind::Wifi || !dev.fallback_slirp() {
        return;
    }
    if *configured || now() < dhcp_started + DHCP_TIMEOUT {
        return;
    }
    apply_static_fallback(iface, mac);
    *configured = true;
    *respaldo = true;
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
    let vigilar = crate::mm::heap::vigilando();
    if vigilar {
        crate::mm::heap::punto("poll-entra");
    }
    try_attach();
    #[cfg(feature = "drv-rtl8169")]
    match rtl8169::poll_link() {
        Some(true) => on_wired_link_up(),
        Some(false) => on_wired_link_down(),
        None => {}
    }
    #[cfg(feature = "drv-e1000e")]
    match e1000e::poll_link() {
        Some(true) => on_wired_link_up(),
        Some(false) => on_wired_link_down(),
        None => {}
    }
    if vigilar {
        crate::mm::heap::punto("poll-nicho");
    }
    let Some(net) = NET.get() else { return };
    let Some(mut n) = net.try_lock() else {
        // Quien se lleva el candado y no lo suelta deja la pila sin sondear: ni
        // avanza una conexión ni vencen los plazos. Se cuenta de vez en cuando
        // para no ahogar la consola.
        let fallos = LOCK_PERDIDO.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
        if fallos == 1 || fallos.is_multiple_of(1000) {
            crate::klogln!("net: poll sin candado ({fallos} veces)");
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
        dhcp_expires_at,
        respaldo,
        user_tcp,
        ..
    } = &mut *n;

    iface.poll(now(), dev, sockets);
    if vigilar {
        crate::mm::heap::punto("poll-rx");
    }
    if let (true, Some(dhcp)) = (*dhcp_enabled, *dhcp) {
        poll_dhcp(
            iface,
            sockets,
            echo,
            ssh,
            dhcp,
            configured,
            dns,
            dhcp_expires_at,
            respaldo,
        );
        try_static_fallback(
            iface,
            *mac,
            *backend,
            dev,
            *dhcp_started,
            configured,
            respaldo,
        );
    }
    poll_tcp_services(sockets, echo, ssh, *configured);
    poll_user_tcp(iface, sockets, user_tcp, *configured);
    iface.poll(now(), dev, sockets);
    // Después del último `poll`: lo que quedara por enviar ya salió.
    user_tcp.purgar_cerrados(sockets);
    if vigilar {
        crate::mm::heap::punto("poll-sale");
    }
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
        let r = tcp_user::listen_start(sockets, entry);
        crate::klogln!(
            "tcp: listen puerto {port} slot {slot} smoltcp={}",
            if r.is_ok() { "ok" } else { "no" }
        );
    }
    Ok(slot)
}

pub fn tcp_connect(remote: soso_abi::SockAddr) -> Result<usize, i64> {
    crate::mm::heap::vigilar();
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

/// Un listener de userspace atiende **las dos** procedencias: el loopback del
/// propio guest (`ask` conecta a 127.0.0.1) y las conexiones externas que
/// entran por la NIC. Antes sólo miraba el loopback, así que ningún servidor
/// HTTP de userspace era alcanzable desde fuera (T54).
pub fn tcp_accept(listener_slot: usize) -> Result<usize, i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    let entry = n
        .user_tcp
        .entries
        .get(listener_slot)
        .and_then(|e| e.as_ref())
        .ok_or(-soso_abi::EBADF)?;
    let loop_listener = entry.loop_listener;
    let role = entry.role;
    let handle = entry.handle;
    // El loopback va primero: es el camino de `ask`, no puede regresionar.
    if loop_listener && loopback::has_pending(listener_slot) {
        drop(n);
        return tcp_accept_loopback(listener_slot);
    }
    if role != tcp_user::TcpRole::Listening && role != tcp_user::TcpRole::Connected {
        return Err(-soso_abi::EINVAL);
    }
    if tcp_user::tiene_conexion(&n.sockets, handle) {
        let NetStack {
            sockets, user_tcp, ..
        } = &mut *n;
        return tcp_user::ceder_establecida(user_tcp, sockets, listener_slot);
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
        return EstadoTcp::Conectado;
    }
    // Mismo criterio para smoltcp: el par mandó su FIN y no queda nada por
    // leer, así que las lecturas deben dar EOF. Esto **no** cierra nuestro
    // lado —`entry.closed` sigue en false y `tcp_try_write` sigue escribiendo—
    // porque el medio cierre (`shutdown(Write)` del cliente HTTP tras mandar
    // la petición) dice «no mando más», no «no quiero respuesta». Antes lo
    // cerraba `poll_entry` y la respuesta del servidor caía en un socket
    // muerto: el cliente veía cero bytes.
    let handle = entry.handle;
    let s = n.sockets.get::<tcp::Socket>(handle);
    if !s.can_recv()
        && matches!(
            s.state(),
            tcp_user::EstadoSmoltcp::CloseWait
                | tcp_user::EstadoSmoltcp::LastAck
                | tcp_user::EstadoSmoltcp::Closing
                | tcp_user::EstadoSmoltcp::Closed
                | tcp_user::EstadoSmoltcp::TimeWait
        )
    {
        return EstadoTcp::Cerrado;
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

/// Resultado de accept al despertar un waiter: `None` = mismo fd; `Some(slot)` = fd nuevo.
pub fn tcp_accept_wake(listener_slot: usize) -> Result<Option<usize>, i64> {
    if !tcp_listener_ready(listener_slot) {
        return Err(-soso_abi::EAGAIN);
    }
    // `tcp_accept` decide ya entre loopback y externa, y las dos devuelven un
    // slot nuevo: el listener se queda escuchando en ambos casos.
    tcp_accept(listener_slot).map(Some)
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

/// Lo que `info` no cuenta: por qué hay o no dirección, de dónde viene, cuánto
/// le queda al lease y a qué DNS se pregunta de verdad.
pub fn detalle() -> soso_abi::NetDetalle {
    let mut out = soso_abi::NetDetalle::default();
    let Some(net) = NET.get() else {
        return out;
    };
    let n = net.lock();
    let enlace = n.backend != BackendKind::Ninguno && n.dev.carrier();
    if enlace {
        out.flags |= soso_abi::NETDET_ENLACE;
    }
    if n.estatica.is_some() {
        out.flags |= soso_abi::NETDET_ESTATICA;
    }
    if n.pref_wifi {
        out.flags |= soso_abi::NETDET_PREF_WIFI;
    }
    out.motivo = if n.backend == BackendKind::Ninguno {
        soso_abi::NET_MOTIVO_SIN_ADAPTADOR
    } else if n.configured {
        if n.estatica.is_some() {
            soso_abi::NET_MOTIVO_ESTATICA
        } else if n.respaldo {
            soso_abi::NET_MOTIVO_RESPALDO
        } else {
            soso_abi::NET_MOTIVO_DHCP
        }
    } else if !enlace && n.backend == BackendKind::Wifi {
        soso_abi::NET_MOTIVO_WIFI_SIN_AUTORIZAR
    } else if !enlace {
        soso_abi::NET_MOTIVO_SIN_ENLACE
    } else {
        soso_abi::NET_MOTIVO_ESPERANDO_DHCP
    };
    if n.configured
        && n.estatica.is_none()
        && let Some(fin) = n.dhcp_expires_at
    {
        let resto_ms = (fin.total_millis() - now().total_millis()).max(0);
        out.flags |= soso_abi::NETDET_LEASE;
        out.lease_secs = u32::try_from(resto_ms / 1000).unwrap_or(u32::MAX);
    }
    for (hueco, servidor) in out.dns.iter_mut().zip(dns::servidores(&n.dns)) {
        *hueco = servidor.octets();
        out.dns_n += 1;
    }
    out
}

/// Texto de `ip` para una pila con dirección (misma redacción en `/bin/ip`).
fn origen_texto(det: &soso_abi::NetDetalle) -> alloc::string::String {
    match det.motivo {
        soso_abi::NET_MOTIVO_ESTATICA => "fija (/etc/red.conf)".into(),
        soso_abi::NET_MOTIVO_RESPALDO => "de respaldo (sin DHCP)".into(),
        _ if det.flags & soso_abi::NETDET_LEASE != 0 => {
            alloc::format!("DHCP, quedan {} s de lease", det.lease_secs)
        }
        _ => "DHCP".into(),
    }
}

/// Texto del comando `ip` (kshell). El binario de userspace imprime lo mismo.
pub fn print_info() {
    let info = info();
    if info.flags & soso_abi::NET_FLAG_PRESENT == 0 {
        println!("ip: sin adaptador de red");
        return;
    }
    let det = detalle();
    let medio = match info.backend {
        soso_abi::NET_BACKEND_WIFI => "wifi",
        _ => "ethernet",
    };
    if info.flags & soso_abi::NET_FLAG_CONFIGURED == 0 {
        match det.motivo {
            soso_abi::NET_MOTIVO_SIN_ENLACE => println!("ip: sin dirección ({medio} sin enlace)"),
            soso_abi::NET_MOTIVO_WIFI_SIN_AUTORIZAR => {
                println!("ip: sin dirección (wifi asociada, sin claves)")
            }
            _ => println!("ip: sin dirección (esperando DHCP)"),
        }
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
    if info.flags & soso_abi::NET_FLAG_CONFIGURED != 0 {
        println!("  dirección {}", origen_texto(&det));
        if det.dns_n > 0 {
            let mut linea = alloc::string::String::from("  dns");
            for (i, d) in det.dns.iter().take(det.dns_n as usize).enumerate() {
                let coma = if i > 0 { "," } else { "" };
                linea.push_str(&alloc::format!("{coma} {}.{}.{}.{}", d[0], d[1], d[2], d[3]));
            }
            println!("{linea}");
        }
    }
    if det.flags & soso_abi::NETDET_PREF_WIFI != 0 {
        println!("  medio: wifi elegido con wifi connect (`ip medio auto` lo suelta)");
    }
}

/// ¿Se puede poner la IP ya? Cable siempre (el DHCP arranca al subir el
/// enlace); WiFi sólo autorizada.
fn enlace_usable(n: &NetStack) -> bool {
    n.backend == BackendKind::Wired || (n.backend == BackendKind::Wifi && wifi_link_up())
}

/// `ip estatica <a.b.c.d/p> [gw <ip>] [dns <ip>]`: guarda `/etc/red.conf` y
/// aplica la dirección. `-EINVAL` si la orden no vale; `-EIO` si no se pudo
/// guardar (entonces no se aplica nada: lo que se ve es lo que arrancará).
pub fn configurar_estatica(texto: &str) -> Result<(), i64> {
    let e = soso_red_core::parsear_orden(texto).map_err(|_| -soso_abi::EINVAL)?;
    if !guardar_red_conf(&soso_red_core::formatear_red_conf(&e)) {
        return Err(-soso_abi::EIO);
    }
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    n.estatica = Some(e);
    if enlace_usable(&n) {
        reiniciar_ip(&mut n);
    }
    Ok(())
}

/// `ip dhcp`: deja `/etc/red.conf` en `modo=dhcp` y vuelve a pedir lease.
pub fn configurar_dhcp() -> Result<(), i64> {
    if !guardar_red_conf(soso_red_core::RED_CONF_DHCP) {
        return Err(-soso_abi::EIO);
    }
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    n.estatica = None;
    n.dns = [None; 3];
    if enlace_usable(&n) {
        reiniciar_ip(&mut n);
    } else {
        // El socket DHCP vuelve a existir; arrancará con el enlace.
        arrancar_dhcp(&mut n);
        n.dhcp_enabled = false;
    }
    Ok(())
}

/// `ip medio auto`: suelta la preferencia por WiFi. Si el cable tiene enlace y
/// no hay dirección fija, la pila vuelve a él en el acto.
pub fn medio_auto() -> Result<(), i64> {
    let net = NET.get().ok_or(-soso_abi::EIO)?;
    let mut n = net.lock();
    n.pref_wifi = false;
    if n.backend == BackendKind::Wifi
        && n.estatica.is_none()
        && let Some((mac, dev, nombre)) = sondear_cable_nativo()
        && dev.carrier()
    {
        cambiar_dispositivo(&mut n, mac, dev, BackendKind::Wired);
        reiniciar_ip(&mut n);
        println!("net: cable con enlace — vuelvo a ethernet ({nombre})");
    }
    Ok(())
}

/// Comando `ip` de la kernel-shell. El binario de userspace habla lo mismo por
/// `SYS_NETCFG`.
pub fn ip_cmd(args: &[&str]) {
    match args {
        [] => print_info(),
        ["estatica", resto @ ..] => {
            let texto = resto.join(" ");
            if let Err(e) = soso_red_core::parsear_orden(&texto) {
                println!("ip: {}", e.texto());
                return;
            }
            match configurar_estatica(&texto) {
                Ok(()) => print_info(),
                Err(e) => println!("ip: no pude guardar {} (errno {})", soso_red_core::RED_CONF, -e),
            }
        }
        ["dhcp"] => match configurar_dhcp() {
            Ok(()) => println!("ip: DHCP (guardado en {})", soso_red_core::RED_CONF),
            Err(e) => println!("ip: no pude guardar {} (errno {})", soso_red_core::RED_CONF, -e),
        },
        ["medio", "auto"] => {
            let _ = medio_auto();
            print_info();
        }
        _ => println!("uso: ip | ip estatica <a.b.c.d/p> [gw <ip>] [dns <ip>] | ip dhcp | ip medio auto"),
    }
}

pub fn tcp_listener_ready(slot: usize) -> bool {
    let Some(net) = NET.get() else { return false };
    // Ver la nota de `tcp_is_connected`.
    let Some(n) = net.try_lock() else { return false };
    let Some(entry) = n.user_tcp.entries.get(slot).and_then(|e| e.as_ref()) else {
        return false;
    };
    // Loopback **o** conexión externa establecida: un listener de userspace
    // sirve a las dos (T54). Un slot puramente de loopback no tiene socket
    // smoltcp en uso; `tiene_conexion` no lo toma por una conexión real.
    if entry.loop_listener && loopback::has_pending(slot) {
        return true;
    }
    tcp_user::tiene_conexion(&n.sockets, entry.handle)
}

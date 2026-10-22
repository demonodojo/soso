//! `ip` — muestra la IPv4 de la máquina y la configura.
//!
//! - `ip`: dirección, pasarela, MAC, DNS y de dónde sale la dirección.
//! - `ip estatica <a.b.c.d/p> [gw <ip>] [dns <ip>]`: dirección fija (se guarda en `/etc/red.conf`).
//! - `ip dhcp`: vuelve a DHCP.
//! - `ip medio auto`: suelta la preferencia por WiFi de `wifi connect`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

const USO: &str = "uso: ip | ip estatica <a.b.c.d/p> [gw <ip>] [dns <ip>] | ip dhcp | ip medio auto";

fn main(args: &[String]) -> u8 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => mostrar(),
        ["estatica", resto @ ..] => {
            let texto = resto.join(" ");
            resultado(sys::net_estatica(&texto))
        }
        ["dhcp"] => resultado(sys::net_dhcp()),
        ["medio", "auto"] => resultado(sys::net_medio_auto()),
        _ => {
            println!("{USO}");
            2
        }
    }
}

/// Ejecuta la orden y, si salió bien, enseña el estado nuevo.
fn resultado(rc: i64) -> u8 {
    if rc >= 0 {
        return mostrar();
    }
    // -EINVAL es una orden mal escrita; el resto, un fallo del sistema.
    if rc == -abi::EINVAL {
        println!("ip: orden no válida\n{USO}");
    } else {
        println!("ip: {}", errno_str(rc));
    }
    1
}

fn mostrar() -> u8 {
    let mut info = abi::NetInfo::default();
    let rc = sys::netinfo(&mut info);
    if rc < 0 {
        println!("ip: {}", errno_str(rc));
        return 1;
    }
    if info.flags & abi::NET_FLAG_PRESENT == 0 {
        println!("ip: sin adaptador de red");
        return 1;
    }
    // Un kernel sin `SYS_NETCFG` deja el detalle a cero: se sigue mostrando lo básico.
    let mut det = abi::NetDetalle::default();
    let _ = sys::net_detalle(&mut det);
    let medio = match info.backend {
        abi::NET_BACKEND_WIFI => "wifi",
        _ => "ethernet",
    };
    let mac = format!(
        "  mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  {medio}",
        info.mac[0], info.mac[1], info.mac[2], info.mac[3], info.mac[4], info.mac[5]
    );
    if info.flags & abi::NET_FLAG_CONFIGURED == 0 {
        match det.motivo {
            abi::NET_MOTIVO_SIN_ENLACE => println!("ip: sin dirección ({medio} sin enlace)"),
            abi::NET_MOTIVO_WIFI_SIN_AUTORIZAR => {
                println!("ip: sin dirección (wifi asociada, sin claves)")
            }
            _ => println!("ip: sin dirección (esperando DHCP)"),
        }
        println!("{mac}");
        return 1;
    }
    if info.gateway != [0, 0, 0, 0] {
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
    println!("{mac}");
    let origen = match det.motivo {
        abi::NET_MOTIVO_ESTATICA => String::from("fija (/etc/red.conf)"),
        abi::NET_MOTIVO_RESPALDO => String::from("de respaldo (sin DHCP)"),
        _ if det.flags & abi::NETDET_LEASE != 0 => {
            format!("DHCP, quedan {} s de lease", det.lease_secs)
        }
        _ => String::from("DHCP"),
    };
    println!("  dirección {origen}");
    if det.dns_n > 0 {
        let lista: Vec<String> = det
            .dns
            .iter()
            .take(det.dns_n as usize)
            .map(|d| format!("{}.{}.{}.{}", d[0], d[1], d[2], d[3]))
            .collect();
        println!("  dns {}", lista.join(", "));
    }
    if det.flags & abi::NETDET_PREF_WIFI != 0 {
        println!("  medio: wifi elegido con wifi connect (`ip medio auto` lo suelta)");
    }
    0
}

//! `ip` — muestra la IPv4 de la máquina (DHCP o fallback).

#![no_std]
#![no_main]

use libsoso::{abi, errno_str, println, sys};

libsoso::entry!(main);

fn main(_args: &str) -> u8 {
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
    let medio = match info.backend {
        abi::NET_BACKEND_WIFI => "wifi",
        _ => "ethernet",
    };
    if info.flags & abi::NET_FLAG_CONFIGURED == 0 {
        println!("ip: sin dirección (esperando DHCP)");
        println!(
            "  mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  {medio}",
            info.mac[0], info.mac[1], info.mac[2], info.mac[3], info.mac[4], info.mac[5]
        );
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
    println!(
        "  mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}  {medio}",
        info.mac[0], info.mac[1], info.mac[2], info.mac[3], info.mac[4], info.mac[5]
    );
    0
}

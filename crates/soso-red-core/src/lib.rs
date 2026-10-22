//! Configuración de red de soso, sin E/S: el kernel lee y escribe los ficheros
//! y este crate decide qué dicen. El banco host prueba lo que dentro del kernel
//! no tiene forma de comprobarse.
//!
//! - [`estatica`]: `/etc/red.conf` (dirección fija) y la orden `ip estatica`.
//! - [`wifi`]: `wifi.conf` / `SOSOWIFI.TXT` con hasta [`wifi::MAX_REDES`] redes.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod estatica;
pub mod wifi;

pub use estatica::{
    formatear_red_conf, parsear_orden, parsear_red_conf, ErrorRed, RedEstatica, RED_CONF,
    RED_CONF_DHCP,
};
pub use wifi::{formatear_redes, parsear_redes, promover, RedWifi, MAX_REDES};

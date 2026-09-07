//! Reloj de pared (RTC CMOS) + tiempo monótono.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

/// Segundos Unix en el instante de calibración (arranque).
static EPOCH_AT_BOOT: AtomicU64 = AtomicU64::new(0);

fn bcd(v: u8) -> u64 {
    ((v >> 4) & 0x0F) as u64 * 10 + (v & 0x0F) as u64
}

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        Port::<u8>::new(0x70).write(reg | 0x80);
        Port::<u8>::new(0x71).read()
    }
}

/// Lee la hora del RTC CMOS y la convierte a segundos Unix (UTC).
pub fn read_epoch_unix() -> u64 {
    let sec = bcd(cmos_read(0x00));
    let min = bcd(cmos_read(0x02));
    let hour = bcd(cmos_read(0x04));
    let day = bcd(cmos_read(0x07));
    let month = bcd(cmos_read(0x08));
    let year = bcd(cmos_read(0x09)) + if cmos_read(0x0B) & 0x04 != 0 {
        2000
    } else {
        1900
    };
    if month == 0 || day == 0 {
        return 0;
    }
    days_since_epoch(year, month, day) * 86400 + hour * 3600 + min * 60 + sec
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    let mut days = 0u64;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let mdays = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        let mut d = mdays[(m - 1) as usize] as u64;
        if m == 2 && is_leap(year) {
            d += 1;
        }
        days += d;
    }
    days + day - 1
}

pub fn init() {
    let epoch = read_epoch_unix();
    EPOCH_AT_BOOT.store(epoch, Ordering::Relaxed);
    if epoch > 0 {
        crate::println!("rtc: epoch Unix {epoch}");
    } else {
        crate::println!("rtc: sin hora CMOS válida; mtime = uptime");
    }
}

/// Segundos de reloj de pared (epoch Unix). Si no hay RTC, uptime.
pub fn wall_secs() -> u64 {
    let base = EPOCH_AT_BOOT.load(Ordering::Relaxed);
    if base == 0 {
        crate::arch::pit::uptime_ms() / 1000
    } else {
        base + crate::arch::pit::uptime_ms() / 1000
    }
}

/// Nanosegundos monótonos desde el arranque.
pub fn monotonic_ns() -> u64 {
    crate::arch::tsc::now_ns()
}

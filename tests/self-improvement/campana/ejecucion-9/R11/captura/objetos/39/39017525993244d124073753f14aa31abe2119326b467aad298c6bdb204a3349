//! Reloj de pared (RTC CMOS) + tiempo monótono.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

/// Segundos Unix en el instante de calibración (arranque).
static EPOCH_AT_BOOT: AtomicU64 = AtomicU64::new(0);

const CMOS_REG_SEC: u8 = 0x00;
const CMOS_REG_MIN: u8 = 0x02;
const CMOS_REG_HOUR: u8 = 0x04;
const CMOS_REG_DAY: u8 = 0x07;
const CMOS_REG_MONTH: u8 = 0x08;
const CMOS_REG_YEAR: u8 = 0x09;
const CMOS_REG_STATUS_A: u8 = 0x0A;
const CMOS_REG_STATUS_B: u8 = 0x0B;
const CMOS_REG_CENTURY: u8 = 0x32;

const STATUS_A_UIP: u8 = 0x80;
const STATUS_B_24H: u8 = 0x02;
const STATUS_B_BINARY: u8 = 0x04;
const HOUR_PM: u8 = 0x80;

fn bcd(v: u8) -> u64 {
    ((v >> 4) & 0x0F) as u64 * 10 + (v & 0x0F) as u64
}

fn decode_reg(raw: u8, binary: bool) -> u64 {
    if binary {
        raw as u64
    } else {
        bcd(raw)
    }
}

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        Port::<u8>::new(0x70).write(reg | 0x80);
        Port::<u8>::new(0x71).read()
    }
}

fn wait_cmos_ready() {
    for _ in 0..1000 {
        if cmos_read(CMOS_REG_STATUS_A) & STATUS_A_UIP == 0 {
            return;
        }
        for _ in 0..10 {
            core::hint::spin_loop();
        }
    }
}

fn read_cmos_snapshot() -> [u8; 8] {
    wait_cmos_ready();
    [
        cmos_read(CMOS_REG_SEC),
        cmos_read(CMOS_REG_MIN),
        cmos_read(CMOS_REG_HOUR),
        cmos_read(CMOS_REG_DAY),
        cmos_read(CMOS_REG_MONTH),
        cmos_read(CMOS_REG_YEAR),
        cmos_read(CMOS_REG_STATUS_A),
        cmos_read(CMOS_REG_STATUS_B),
    ]
}

fn decode_hour(raw: u8, binary: bool, hour24: bool) -> u64 {
    let mut hour = decode_reg(raw, binary);
    if !hour24 {
        let pm = raw & HOUR_PM != 0;
        hour = decode_reg(raw & !HOUR_PM, binary);
        if hour == 12 {
            hour = if pm { 12 } else { 0 };
        } else if pm {
            hour += 12;
        }
    }
    hour
}

fn decode_year(year_yy: u64, century_raw: u8, binary: bool) -> u64 {
    let century = decode_reg(century_raw, binary);
    if (19..=21).contains(&century) {
        century as u64 * 100 + year_yy
    } else {
        2000 + year_yy
    }
}

/// Convierte campos CMOS decodificados a segundos Unix (UTC).
pub fn epoch_from_fields(
    sec: u64,
    min: u64,
    hour: u64,
    day: u64,
    month: u64,
    year: u64,
) -> u64 {
    if year < 1970 || month == 0 || month > 12 || day == 0 {
        return 0;
    }
    days_since_epoch(year, month, day) * 86400 + hour * 3600 + min * 60 + sec
}

/// Lee la hora del RTC CMOS y la convierte a segundos Unix (UTC).
pub fn read_epoch_unix() -> u64 {
    let snap = read_cmos_snapshot();
    let status_b = snap[7];
    let binary = status_b & STATUS_B_BINARY != 0;
    let hour24 = status_b & STATUS_B_24H != 0;

    let sec = decode_reg(snap[0], binary);
    let min = decode_reg(snap[1], binary);
    let hour = decode_hour(snap[2], binary, hour24);
    let day = decode_reg(snap[3], binary);
    let month = decode_reg(snap[4], binary);
    let year_yy = decode_reg(snap[5], binary);
    let century_raw = cmos_read(CMOS_REG_CENTURY);
    let year = decode_year(year_yy, century_raw, binary);

    epoch_from_fields(sec, min, hour, day, month, year)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
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

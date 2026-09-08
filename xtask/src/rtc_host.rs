//! Tests host del decode CMOS (espejo de kernel/src/arch/rtc.rs).

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

fn decode_hour(raw: u8, binary: bool, hour24: bool) -> u64 {
    let mut hour = decode_reg(raw, binary);
    if !hour24 {
        let pm = raw & 0x80 != 0;
        hour = decode_reg(raw & !0x80, binary);
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

fn epoch_from_fields(sec: u64, min: u64, hour: u64, day: u64, month: u64, year: u64) -> u64 {
    if year < 1970 || month == 0 || month > 12 || day == 0 {
        return 0;
    }
    days_since_epoch(year, month, day) * 86400 + hour * 3600 + min * 60 + sec
}

fn epoch_from_bcd_regs(
    sec: u8,
    min: u8,
    hour: u8,
    day: u8,
    month: u8,
    year_yy: u8,
    status_b: u8,
    century: u8,
) -> u64 {
    let binary = status_b & 0x04 != 0;
    let hour24 = status_b & 0x02 != 0;
    let y = decode_year(decode_reg(year_yy, binary), century, binary);
    epoch_from_fields(
        decode_reg(sec, binary),
        decode_reg(min, binary),
        decode_hour(hour, binary, hour24),
        decode_reg(day, binary),
        decode_reg(month, binary),
        y,
    )
}

pub fn run_rtc_host_tests() -> Result<(), String> {
    // 2026-09-07 12:00:00 UTC, BCD + century 0x20 → 2026 (no 1900+0x26).
    let epoch = epoch_from_bcd_regs(0x00, 0x00, 0x12, 0x07, 0x09, 0x26, 0x02, 0x20);
    if epoch < 1_000_000_000 {
        return Err(format!("epoch 2026 demasiado bajo: {epoch}"));
    }
    let expect = epoch_from_fields(0, 0, 12, 7, 9, 2026);
    if epoch != expect {
        return Err(format!("epoch 2026-09-07 12:00: got {epoch} expect {expect}"));
    }

    // Antes: Status B bit 2 (Data Mode) se confundía con siglo → 1900+26 → ~1970-09-07.
    let legacy_year = bcd(0x26) + 1900;
    let legacy_epoch = days_since_epoch(legacy_year, 9, 7) * 86400 + 12 * 3600;
    if legacy_epoch >= 1_000_000_000 {
        return Err(format!("regresión century bit2: legacy={legacy_epoch}"));
    }
    if legacy_epoch < 20_000_000 || legacy_epoch > 25_000_000 {
        return Err(format!(
            "legacy epoch fuera de rango esperado (~1970): {legacy_epoch}"
        ));
    }
    if epoch <= legacy_epoch {
        return Err(format!("epoch corregido no supera legacy: {epoch} vs {legacy_epoch}"));
    }

    // 12 h PM: 01:30:00 PM = 13:30:00
    let pm = epoch_from_bcd_regs(0x00, 0x30, 0x81, 0x07, 0x09, 0x26, 0x00, 0x20);
    let expect_pm = epoch_from_fields(0, 30, 13, 7, 9, 2026);
    if pm != expect_pm {
        return Err(format!("12h PM: got {pm} expect {expect_pm}"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run_rtc_host_tests;

    #[test]
    fn rtc_decode_2026() {
        run_rtc_host_tests().expect("rtc host");
    }
}

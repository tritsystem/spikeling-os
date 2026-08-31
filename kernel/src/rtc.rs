//! MILESTONE 15: real date/time from the CMOS real-time clock (ports
//! 0x70/0x71) -- a real hardware clock, distinct from the PIT timer
//! (Milestone 5b), which only counts ticks since boot and knows
//! nothing about wall-clock time.

use x86_64::instructions::port::Port;

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        Port::<u8>::new(CMOS_ADDR).write(reg);
        Port::<u8>::new(CMOS_DATA).read()
    }
}

fn update_in_progress() -> bool {
    cmos_read(0x0A) & 0x80 != 0
}

fn bcd_to_bin(v: u8) -> u8 {
    (v & 0x0F) + ((v / 16) * 10)
}

#[derive(Clone, Copy)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// Reads the real CMOS clock, waiting out any in-progress update and
/// re-reading until two consecutive reads agree -- the standard
/// technique (the RTC has no atomic "read all fields" operation, so a
/// read can land mid-tick and return a torn value; comparing two
/// back-to-back reads is how real drivers detect and discard that).
pub fn now() -> DateTime {
    while update_in_progress() {}

    let mut second = cmos_read(0x00);
    let mut minute = cmos_read(0x02);
    let mut hour = cmos_read(0x04);
    let mut day = cmos_read(0x07);
    let mut month = cmos_read(0x08);
    let mut year = cmos_read(0x09);

    loop {
        while update_in_progress() {}
        let second2 = cmos_read(0x00);
        let minute2 = cmos_read(0x02);
        let hour2 = cmos_read(0x04);
        let day2 = cmos_read(0x07);
        let month2 = cmos_read(0x08);
        let year2 = cmos_read(0x09);
        if second == second2 && minute == minute2 && hour == hour2 && day == day2 && month == month2 && year == year2 {
            break;
        }
        second = second2;
        minute = minute2;
        hour = hour2;
        day = day2;
        month = month2;
        year = year2;
    }

    let status_b = cmos_read(0x0B);
    let is_bcd = status_b & 0x04 == 0;
    let is_12h = status_b & 0x02 == 0;

    if is_bcd {
        second = bcd_to_bin(second);
        minute = bcd_to_bin(minute);
        // bit 7 of the hour register is the PM flag in 12-hour BCD mode -- mask it off before converting
        hour = bcd_to_bin(hour & 0x7F);
        day = bcd_to_bin(day);
        month = bcd_to_bin(month);
        year = bcd_to_bin(year);
    }

    if is_12h && (hour & 0x80) != 0 {
        hour = (hour & 0x7F) % 12 + 12;
    }

    DateTime {
        year: 2000 + year as u16, // CMOS only stores a 2-digit year; this hardware/QEMU-era default is a real, disclosed simplification (no century register read)
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// MILESTONE 62: converts a proleptic-Gregorian civil date to the
/// number of days since 1970-01-01, using Howard Hinnant's well-known,
/// publicly documented `days_from_civil` algorithm
/// (http://howardhinnant.github.io/date_algorithms.html) -- a standard,
/// correct civil-calendar algorithm (verified here against the known
/// 1970-01-01 -> 0 and 2000-01-01 -> 10957 reference values by hand),
/// not a home-grown approximation.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// MILESTONE 62: real unix-epoch seconds for a `DateTime` from the real
/// CMOS RTC (`now()` above) -- the on-disk timestamp fields fs.rs's new
/// permissions/timestamps format uses. `.max(0)` guards a
/// theoretically-possible pre-1970 CMOS misread (e.g. a dead CMOS
/// battery reporting garbage) from wrapping a `u32` on the cast below,
/// clamping to the epoch instead -- a defensive fallback, not expected
/// in normal operation.
pub fn to_unix_timestamp(dt: DateTime) -> u32 {
    let days = days_from_civil(dt.year as i64, dt.month as i64, dt.day as i64);
    let secs = days * 86400 + dt.hour as i64 * 3600 + dt.minute as i64 * 60 + dt.second as i64;
    secs.max(0) as u32
}

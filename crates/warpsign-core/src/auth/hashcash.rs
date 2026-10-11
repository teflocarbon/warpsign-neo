//! The `X-APPLE-HC` proof-of-work header Apple requires on sign-in since
//! February 2023. Without it, sign-in fails with 403 and can lock the Apple
//! ID. Same algorithm as fastlane's `Spaceship::Hashcash.make`.

use std::time::{SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};

/// Build the header value for the `X-Apple-HC-Bits` and
/// `X-Apple-HC-Challenge` values Apple sent, stamped with the current time
pub fn make(bits: u32, challenge: &str) -> String {
    make_at(bits, challenge, &timestamp(SystemTime::now()))
}

/// Build the header value with a given `YYYYMMDDHHMMSS` timestamp
pub fn make_at(bits: u32, challenge: &str, date: &str) -> String {
    // Apple's variant: no random part, and the counter is plain decimal
    // (hence the double colon)
    (0u64..)
        .map(|counter| format!("1:{bits}:{date}:{challenge}::{counter}"))
        .find(|candidate| leading_zero_bits(&Sha1::digest(candidate.as_bytes())) >= bits)
        .expect("a counter is always found eventually")
}

fn leading_zero_bits(digest: &[u8]) -> u32 {
    let mut zeros = 0;
    for byte in digest {
        zeros += byte.leading_zeros();
        if *byte != 0 {
            break;
        }
    }
    zeros
}

/// `YYYYMMDDHHMMSS` in UTC
pub fn timestamp(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day), from Howard Hinnant's
/// `civil_from_days`
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

//! Clock and identifier source. The core never reads the system clock or a
//! random source itself (neither exists in plain WebAssembly); callers supply them.

pub trait Env {
    /// Current time, RFC 3339 UTC with milliseconds.
    fn now(&mut self) -> String;
    /// A new unique identifier with the given prefix, e.g. `st_…`.
    fn new_id(&mut self, prefix: &str) -> String;
}

/// Deterministic environment for tests and reproducible fixtures.
#[derive(Clone, Debug, Default)]
pub struct FixedEnv {
    pub counter: u64,
}

impl Env for FixedEnv {
    fn now(&mut self) -> String {
        self.counter += 1;
        format_rfc3339_ms(1_789_000_000_000 + self.counter * 1000)
    }

    fn new_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}_{:026}", self.counter)
    }
}

/// Milliseconds since the Unix epoch → `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_rfc3339_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let millis = ms % 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// ULID: 48-bit millisecond timestamp + 80 random bits, Crockford base32.
/// Sorts by creation time.
pub fn ulid(ms: u64, random: [u8; 10]) -> String {
    let mut value: u128 = (ms as u128 & 0xFFFF_FFFF_FFFF) << 80;
    for (i, b) in random.iter().enumerate() {
        value |= (*b as u128) << (72 - 8 * i);
    }
    let mut out = [0u8; 26];
    for i in (0..26).rev() {
        out[i] = CROCKFORD[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("ascii")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_dates() {
        assert_eq!(format_rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_rfc3339_ms(951_782_400_123),
            "2000-02-29T00:00:00.123Z"
        );
        assert_eq!(
            format_rfc3339_ms(1_790_208_000_000),
            "2026-09-24T00:00:00.000Z"
        );
    }

    #[test]
    fn ulids_sort_by_time() {
        let a = ulid(1_000, [255; 10]);
        let b = ulid(1_001, [0; 10]);
        assert_eq!(a.len(), 26);
        assert!(a < b);
    }
}

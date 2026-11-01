//! Calendar math without a date crate (UTC, proleptic Gregorian).

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's civil calendar algorithm.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// Inverse of `civil_from_days`.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// "2026-10-20 21:55:04.800" -> seconds since 1970-01-01 (no time zone; only differences matter).
pub fn parse_timestamp(s: &str) -> Option<f64> {
    let (date, clock) = s.trim().split_once(' ')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut c = clock.split(':');
    let (h, min) = (c.next()?.parse::<f64>().ok()?, c.next()?.parse::<f64>().ok()?);
    let sec = c.next()?.parse::<f64>().ok()?;
    Some(days_from_civil(y, m, day) as f64 * 86_400.0 + h * 3600.0 + min * 60.0 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_timestamps() {
        for days in [-1, 0, 11_016, 20_747] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        let a = parse_timestamp("2026-10-20 23:59:58.500").unwrap();
        let b = parse_timestamp("2026-10-21 00:00:03.000").unwrap();
        assert!((b - a - 4.5).abs() < 1e-6, "midnight rollover");
        assert_eq!(parse_timestamp("garbage"), None);
    }
}

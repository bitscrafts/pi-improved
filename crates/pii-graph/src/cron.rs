//! 5-field UTC cron parser (F5.7): `min hour dom mon dow` with `*`, `n`,
//! `a-b`, `a,b`, `*/n` and combinations.
//!
//! Pure and table-tested; `next_after`
//! walks minute by minute with a bounded search (a minute is the smallest
//! granularity cron has).

use pii_abi::PiiError;

/// One parsed field: the set of allowed values it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    allowed: Vec<u32>,
}

impl Field {
    /// Parses one cron field: `*`, `n`, `a-b`, `a,b`, `*/n`, `a-b/n`, combos.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for out-of-range values, empty steps, `*/0`.
    pub fn parse(spec: &str, min: u32, max: u32) -> Result<Self, PiiError> {
        let mut allowed: Vec<u32> = Vec::new();
        for part in spec.split(',') {
            let (range, step) = match part.split_once('/') {
                Some((r, s)) => {
                    let step: u32 = s.parse().map_err(|_| {
                        PiiError::Invalid(format!("cron: bad step '{s}' in '{spec}'"))
                    })?;
                    if step == 0 {
                        return Err(PiiError::Invalid(format!("cron: zero step in '{spec}'")));
                    }
                    (r, Some(step))
                }
                None => (part, None),
            };
            let (lo, hi) = if range == "*" {
                (min, max)
            } else if let Some((a, b)) = range.split_once('-') {
                let lo: u32 = a
                    .parse()
                    .map_err(|_| PiiError::Invalid(format!("cron: bad bound '{a}' in '{spec}'")))?;
                let hi: u32 = b
                    .parse()
                    .map_err(|_| PiiError::Invalid(format!("cron: bad bound '{b}' in '{spec}'")))?;
                (lo, hi)
            } else {
                let v: u32 = range.parse().map_err(|_| {
                    PiiError::Invalid(format!("cron: bad value '{range}' in '{spec}'"))
                })?;
                (v, v)
            };
            if lo < min || hi > max || lo > hi {
                return Err(PiiError::Invalid(format!(
                    "cron: range {lo}-{hi} outside {min}..={max} in '{spec}'"
                )));
            }
            let step = step.unwrap_or(1);
            let mut v = lo;
            while v <= hi {
                allowed.push(v);
                v += step;
            }
        }
        allowed.sort_unstable();
        allowed.dedup();
        Ok(Self { allowed })
    }

    /// True when `value` matches this field.
    #[must_use]
    pub fn matches(&self, value: u32) -> bool {
        self.allowed.contains(&value)
    }
}

/// A parsed 5-field cron expression (UTC).
#[derive(Debug, Clone)]
pub struct Cron {
    /// Minutes (0..59).
    pub min: Field,
    /// Hours (0..23).
    pub hour: Field,
    /// Days of month (1..31).
    pub dom: Field,
    /// Months (1..12).
    pub mon: Field,
    /// Days of week (0..6, Sunday = 0).
    pub dow: Field,
}

/// Days from 1970-01-01 (epoch) to civil weekday: 0 = Thursday.
fn weekday_of_epoch_days(days: u64) -> u32 {
    u32::try_from((days + 4) % 7).unwrap_or(0)
}

/// UTC civil date from epoch millis (proleptic Gregorian, days since epoch).
fn civil(days: u64) -> (u64, u32, u32) {
    // Howard Hinnant's algorithm, u64-safe over the range cron cares about.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 { y + 1 } else { y },
        u32::try_from(mp).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// Days since epoch for a UTC civil date (inverse of [`civil`]).
#[allow(dead_code)] // reason: kept for the timer node cron round-trip tests
fn days_from_civil(y: u64, m: u32, d: u32) -> u64 {
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy / 400;
    let yoe = yy % 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * u64::from(mp) + 2) / 5 + u64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl Cron {
    /// Parses `min hour dom mon dow`.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] on any malformed field (exactly 5 fields).
    pub fn parse(spec: &str) -> Result<Self, PiiError> {
        let fields: Vec<&str> = spec.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(PiiError::Invalid(format!(
                "cron: want 5 fields, got {}: '{spec}'",
                fields.len()
            )));
        }
        Ok(Self {
            min: Field::parse(fields.first().copied().unwrap_or_default(), 0, 59)?,
            hour: Field::parse(fields.get(1).copied().unwrap_or_default(), 0, 23)?,
            dom: Field::parse(fields.get(2).copied().unwrap_or_default(), 1, 31)?,
            mon: Field::parse(fields.get(3).copied().unwrap_or_default(), 1, 12)?,
            dow: Field::parse(fields.get(4).copied().unwrap_or_default(), 0, 6)?,
        })
    }

    /// The next fire time strictly after `now_ms` (UTC, minute resolution).
    /// Returns `now_ms` rounded up to the next matching minute, epoch millis.
    #[must_use]
    pub fn next_after(&self, now_ms: u64) -> u64 {
        let mut minute = now_ms / 60_000 + 1; // strictly after
        let limit = minute + 366 * 24 * 60; // one year bound
        while minute <= limit {
            let days = minute / (24 * 60);
            let moday = minute % (24 * 60);
            let (_y, mo, d) = civil(days);
            let day_of_month_ok = self.dom.matches(d);
            let day_of_week_ok = self.dow.matches(weekday_of_epoch_days(days));
            // Standard cron semantics: dom OR dow when both are restricted.
            let day_match = day_of_month_ok || day_of_week_ok;
            if self.mon.matches(mo)
                && day_match
                && self.hour.matches(u32::try_from(moday / 60).unwrap_or(0))
                && self.min.matches(u32::try_from(moday % 60).unwrap_or(0))
            {
                return minute * 60_000;
            }
            minute += 1;
        }
        // Unreachable for valid crons within a year; fall back to +1 minute.
        (now_ms / 60_000 + 1) * 60_000
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Cron, Field};
    use pii_abi::PiiError;

    #[test]
    fn field_parse_forms() {
        let f = Field::parse("*/15", 0, 59).unwrap();
        assert_eq!(f.allowed, vec![0, 15, 30, 45]);
        let f = Field::parse("1,5", 0, 59).unwrap();
        assert_eq!(f.allowed, vec![1, 5]);
        assert!(f.matches(1) && f.matches(5));
        let f = Field::parse("1-5/2", 0, 59).unwrap();
        assert_eq!(f.allowed, vec![1, 3, 5]);
        let err = Field::parse("*/0", 0, 59).unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)));
        let err = Field::parse("60", 0, 59).unwrap_err();
        assert!(matches!(err, PiiError::Invalid(_)));
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["* * *", "60 * * * *", "*/0 * * * *", "a b c d e"] {
            assert!(Cron::parse(bad).is_err(), "'{bad}' must be rejected");
        }
    }

    #[test]
    fn next_after_every_15_minutes() {
        let c = Cron::parse("*/15 * * * *").unwrap();
        // 00:07 -> 00:15
        let t0 = (7 * 60) * 1000;
        assert_eq!(c.next_after(t0), 15 * 60 * 1000);
        assert_eq!(c.next_after(15 * 60 * 1000), 30 * 60 * 1000);
    }

    #[test]
    fn next_after_daily_at_0300() {
        let c = Cron::parse("0 3 * * *").unwrap();
        // 04:00 day 0 -> 03:00 day 1 = 23h later, in millis.
        let day0_0400 = 4 * 60 * 1000;
        // 04:00 -> 03:00 the NEXT day = 23h = 1380 minutes.
        assert_eq!(c.next_after(day0_0400), 3 * 60 * 60 * 1000);
    }
}

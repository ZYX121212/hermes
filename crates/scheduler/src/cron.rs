// crates/scheduler/src/cron.rs
// Simple 5-field cron parser and next-fire-time calculator.
use chrono::{Datelike, Timelike};

/// A parsed cron expression with 5 fields: minute, hour, day-of-month, month, day-of-week.
#[derive(Debug, Clone)]
pub struct CronSchedule {
    minutes: CronField,
    hours: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
}

#[derive(Debug, Clone)]
enum CronField {
    Any,
    Single(u32),
    List(Vec<u32>),
    Range(u32, u32),
    Step(u32, u32), // start, step (with * as start)
}

impl CronField {
    fn parse(s: &str, min: u32, _max: u32) -> Result<Self, String> {
        if s == "*" {
            return Ok(CronField::Any);
        }
        // */N step
        if let Some(rest) = s.strip_prefix("*/") {
            let step: u32 = rest.parse().map_err(|_| format!("invalid step: {rest}"))?;
            return Ok(CronField::Step(min, step));
        }
        // N-M range
        if let Some((a, b)) = s.split_once('-') {
            let lo: u32 = a.parse().map_err(|_| format!("invalid range start: {a}"))?;
            let hi: u32 = b.parse().map_err(|_| format!("invalid range end: {b}"))?;
            return Ok(CronField::Range(lo, hi));
        }
        // comma-separated list
        if s.contains(',') {
            let vals: Result<Vec<u32>, _> = s.split(',').map(|v| v.parse::<u32>()).collect();
            return Ok(CronField::List(
                vals.map_err(|e| format!("invalid list: {e}"))?,
            ));
        }
        // single value
        s.parse::<u32>()
            .map(CronField::Single)
            .map_err(|_| format!("invalid cron field: {s}"))
    }

    fn matches(&self, val: u32) -> bool {
        match self {
            CronField::Any => true,
            CronField::Single(v) => *v == val,
            CronField::List(vals) => vals.contains(&val),
            CronField::Range(lo, hi) => *lo <= val && val <= *hi,
            CronField::Step(start, step) => *step > 0 && val >= *start && (val - start) % step == 0,
        }
    }
}

impl CronSchedule {
    /// Parse a 5-field cron expression: "min hour dom month dow"
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!("cron 表达式需要 5 个字段，得到 {}", parts.len()));
        }
        Ok(Self {
            minutes: CronField::parse(parts[0], 0, 59)?,
            hours: CronField::parse(parts[1], 0, 23)?,
            dom: CronField::parse(parts[2], 1, 31)?,
            month: CronField::parse(parts[3], 1, 12)?,
            dow: CronField::parse(parts[4], 0, 6)?,
        })
    }

    /// Check if the given UTC datetime matches this schedule.
    pub fn matches(&self, dt: &chrono::DateTime<chrono::Utc>) -> bool {
        let dom = dt.day();
        let dow = dt.weekday().num_days_from_sunday();
        self.minutes.matches(dt.minute())
            && self.hours.matches(dt.hour())
            && self.dom.matches(dom)
            && self.month.matches(dt.month())
            && self.dow.matches(dow)
    }

    /// Compute seconds until the next match from `now`.
    /// Simple implementation: check each minute for up to 2 years.
    pub fn next_in_secs(&self, now: &chrono::DateTime<chrono::Utc>) -> Option<i64> {
        let mut candidate = *now + chrono::Duration::seconds(60 - now.second() as i64 % 60);
        candidate = candidate.with_nanosecond(0).unwrap_or(candidate);

        // Search forward, checking each minute for up to ~2 years
        let deadline = *now + chrono::Duration::days(365 * 2);
        while candidate <= deadline {
            if self.matches(&candidate) && candidate > *now {
                return Some((candidate - *now).num_seconds());
            }
            candidate += chrono::Duration::minutes(1);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_every_minute() {
        let s = CronSchedule::parse("* * * * *").unwrap();
        let now = chrono::Utc::now();
        assert!(s.matches(&now));
    }

    #[test]
    fn test_parse_specific_time() {
        let s = CronSchedule::parse("30 9 * * 1").unwrap(); // 9:30 on Monday
        let dt = chrono::DateTime::parse_from_rfc3339("2026-05-25T09:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(s.matches(&dt), "5/25/2026 is a Monday");
    }

    #[test]
    fn test_parse_range() {
        let s = CronSchedule::parse("0 9-17 * * *").unwrap();
        let dt = chrono::DateTime::parse_from_rfc3339("2026-05-25T14:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(s.matches(&dt));
        let dt2 = chrono::DateTime::parse_from_rfc3339("2026-05-25T08:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(!s.matches(&dt2));
    }

    #[test]
    fn test_parse_step() {
        let s = CronSchedule::parse("*/5 * * * *").unwrap(); // every 5 minutes
        let dt = chrono::DateTime::parse_from_rfc3339("2026-05-25T10:05:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(s.matches(&dt));
        let dt2 = chrono::DateTime::parse_from_rfc3339("2026-05-25T10:03:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(!s.matches(&dt2));
    }

    #[test]
    fn test_next_in_secs_returns_future() {
        let s = CronSchedule::parse("* * * * *").unwrap();
        let now = chrono::Utc::now();
        let secs = s.next_in_secs(&now);
        assert!(secs.is_some());
        assert!(secs.unwrap() > 0, "next fire should be in the future");
    }

    #[test]
    fn test_invalid_expr() {
        assert!(CronSchedule::parse("invalid").is_err());
        assert!(CronSchedule::parse("* * * *").is_err());
        assert!(CronSchedule::parse("* * * * * *").is_err());
    }
}

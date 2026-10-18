//! Shop-local time. The demo shop is in Jakarta (WIB, UTC+7, no DST), and
//! every "today", "Thursday" or "Morning, Rima." is decided here rather than in
//! the browser.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike, Utc, Weekday};

pub fn wib() -> FixedOffset {
    FixedOffset::east_opt(7 * 3600).expect("UTC+7 is a valid offset")
}

pub fn now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&wib())
}

pub fn today() -> NaiveDate {
    now().date_naive()
}

pub fn greeting(owner: &str) -> String {
    let part = match now().hour() {
        4..=10 => "Morning",
        11..=14 => "Afternoon",
        15..=18 => "Evening",
        _ => "Good evening",
    };
    format!("{part}, {owner}.")
}

pub fn weekday_name(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// "today", "tomorrow", a weekday within the week, otherwise "12 Oct".
pub fn relative_day(d: NaiveDate, today: NaiveDate) -> String {
    match (d - today).num_days() {
        0 => "today".into(),
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        2..=6 => weekday_name(d).into(),
        _ => d.format("%-d %b").to_string(),
    }
}

/// "07.04" — the clock style used across the app.
pub fn hhmm() -> String {
    let n = now();
    format!("{:02}.{:02}", n.hour(), n.minute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_day_reads_naturally() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(); // a Monday
        assert_eq!(relative_day(today, today), "today");
        assert_eq!(relative_day(today.succ_opt().unwrap(), today), "tomorrow");
        let thu = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        assert_eq!(relative_day(thu, today), "Thursday");
        let later = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        assert_eq!(relative_day(later, today), "30 Oct");
    }
}

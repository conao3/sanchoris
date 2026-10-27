use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc, Weekday};
use sanchoris_config::schedule::{self, CronExpression, Schedule, ScheduleError};

const SEARCH_DAYS: i64 = 366 * 8;
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NextFireError {
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
    #[error("the cron expression never fires after {0}")]
    Never(DateTime<Utc>),
}

pub fn next_fire(expression: &str, after: DateTime<Utc>) -> Result<DateTime<Utc>, NextFireError> {
    match schedule::parse(expression)? {
        Schedule::Rate { value, unit } => Ok(after
            + Duration::seconds(i64::try_from(value * unit.seconds()).expect("rate fits in i64"))),
        Schedule::Cron(cron) => next_cron(&cron, after),
    }
}

struct Field {
    allowed: Vec<bool>,
    min: u32,
}

impl Field {
    fn parse(text: &str, min: u32, max: u32, names: &[&str]) -> Field {
        let mut allowed = vec![false; (max - min + 1) as usize];
        if text == "?" {
            allowed.iter_mut().for_each(|a| *a = true);
            return Field { allowed, min };
        }
        let atom = |s: &str| -> u32 {
            s.parse::<u32>().unwrap_or_else(|_| {
                let index = names
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(s))
                    .expect("validated cron atom");
                min + u32::try_from(index).expect("small index")
            })
        };
        for item in text.split(',') {
            let (range, step) = match item.split_once('/') {
                Some((r, s)) => (r, s.parse::<u32>().expect("validated cron step")),
                None => (item, 1),
            };
            let (start, end) = if range == "*" {
                (min, max)
            } else if let Some((a, b)) = range.split_once('-') {
                (atom(a), atom(b))
            } else if item.contains('/') {
                (atom(range), max)
            } else {
                (atom(range), atom(range))
            };
            let mut value = start;
            while value <= end {
                allowed[(value - min) as usize] = true;
                value += step;
            }
        }
        Field { allowed, min }
    }

    fn contains(&self, value: u32) -> bool {
        value
            .checked_sub(self.min)
            .and_then(|i| self.allowed.get(i as usize))
            .copied()
            .unwrap_or(false)
    }
}

fn last_day(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .expect("valid date")
        .pred_opt()
        .expect("valid date")
        .day()
}

fn cron_weekday(day: NaiveDate) -> u32 {
    day.weekday().num_days_from_sunday() + 1
}

fn day_of_month_matches(text: &str, day: NaiveDate) -> bool {
    let last = last_day(day.year(), day.month());
    if text == "L" {
        return day.day() == last;
    }
    if let Some(target) = text.strip_suffix('W') {
        let target: u32 = target.parse().expect("validated W day");
        let target = target.min(last);
        let date = NaiveDate::from_ymd_opt(day.year(), day.month(), target).expect("valid date");
        let nearest = match date.weekday() {
            Weekday::Sat if target == 1 => target + 2,
            Weekday::Sat => target - 1,
            Weekday::Sun if target == last => target - 2,
            Weekday::Sun => target + 1,
            _ => target,
        };
        return day.day() == nearest;
    }
    Field::parse(text, 1, 31, &[]).contains(day.day())
}

fn day_of_week_matches(text: &str, day: NaiveDate) -> bool {
    let weekday_of = |s: &str| -> u32 {
        s.parse::<u32>().unwrap_or_else(|_| {
            1 + u32::try_from(
                WEEKDAYS
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(s))
                    .expect("validated weekday"),
            )
            .expect("small index")
        })
    };
    if let Some(name) = text.strip_suffix('L').filter(|s| !s.is_empty()) {
        return cron_weekday(day) == weekday_of(name)
            && day.day() + 7 > last_day(day.year(), day.month());
    }
    if let Some((name, nth)) = text.split_once('#') {
        let nth: u32 = nth.parse().expect("validated nth");
        return cron_weekday(day) == weekday_of(name) && (day.day() - 1) / 7 + 1 == nth;
    }
    Field::parse(text, 1, 7, &WEEKDAYS).contains(cron_weekday(day))
}

fn next_cron(cron: &CronExpression, after: DateTime<Utc>) -> Result<DateTime<Utc>, NextFireError> {
    let minutes = Field::parse(&cron.minutes, 0, 59, &[]);
    let hours = Field::parse(&cron.hours, 0, 23, &[]);
    let months = Field::parse(&cron.month, 1, 12, &MONTHS);
    let years = Field::parse(&cron.year, 1970, 2199, &[]);
    let start = after
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .expect("valid time")
        + Duration::minutes(1);
    let first_day = start.date_naive();
    for offset in 0..SEARCH_DAYS {
        let day = first_day + Duration::days(offset);
        if !years.contains(u32::try_from(day.year()).unwrap_or(0)) || !months.contains(day.month())
        {
            continue;
        }
        let day_ok = if cron.day_of_month == "?" {
            day_of_week_matches(&cron.day_of_week, day)
        } else {
            day_of_month_matches(&cron.day_of_month, day)
        };
        if !day_ok {
            continue;
        }
        for hour in 0..24 {
            if !hours.contains(hour) {
                continue;
            }
            for minute in 0..60 {
                if !minutes.contains(minute) {
                    continue;
                }
                let candidate =
                    Utc.from_utc_datetime(&day.and_hms_opt(hour, minute, 0).expect("valid time"));
                if candidate >= start {
                    return Ok(candidate);
                }
            }
        }
    }
    Err(NextFireError::Never(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn rate_adds_the_interval() {
        assert_eq!(
            next_fire("rate(5 minutes)", at("2026-10-27T09:00:30Z")).unwrap(),
            at("2026-10-27T09:05:30Z")
        );
    }

    #[test]
    fn cron_finds_the_next_minute() {
        let after = at("2026-10-27T09:00:30Z");
        assert_eq!(
            next_fire("cron(0 10 * * ? *)", after).unwrap(),
            at("2026-10-27T10:00:00Z")
        );
        assert_eq!(
            next_fire("cron(0/15 * * * ? *)", after).unwrap(),
            at("2026-10-27T09:15:00Z")
        );
        assert_eq!(
            next_fire("cron(0 9 ? * MON-FRI *)", at("2026-10-30T09:00:00Z")).unwrap(),
            at("2026-11-02T09:00:00Z")
        );
        assert_eq!(
            next_fire("cron(0 0 L * ? *)", after).unwrap(),
            at("2026-10-31T00:00:00Z")
        );
        assert_eq!(
            next_fire("cron(0 0 ? * 6L *)", after).unwrap(),
            at("2026-10-30T00:00:00Z")
        );
        assert_eq!(
            next_fire("cron(0 0 ? * 2#1 *)", after).unwrap(),
            at("2026-11-02T00:00:00Z")
        );
        assert_eq!(
            next_fire("cron(0 0 1W * ? *)", at("2026-10-27T00:00:00Z")).unwrap(),
            at("2026-11-02T00:00:00Z")
        );
        assert_eq!(
            next_fire("cron(0 0 1 1 ? 2020)", after).unwrap_err(),
            NextFireError::Never(after)
        );
    }
}

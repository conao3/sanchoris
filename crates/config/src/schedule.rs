use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    Rate { value: u64, unit: RateUnit },
    Cron(CronExpression),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateUnit {
    Minute,
    Hour,
    Day,
}

impl RateUnit {
    pub fn seconds(self) -> u64 {
        match self {
            RateUnit::Minute => 60,
            RateUnit::Hour => 3600,
            RateUnit::Day => 86400,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpression {
    pub minutes: String,
    pub hours: String,
    pub day_of_month: String,
    pub month: String,
    pub day_of_week: String,
    pub year: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScheduleError {
    #[error("schedule expression must be cron(...) or rate(...)")]
    Syntax,
    #[error("rate(...) must be rate(<positive integer> <minute|minutes|hour|hours|day|days>)")]
    Rate,
    #[error("rate value 1 takes a singular unit and other values take a plural unit")]
    RateUnitNumber,
    #[error("cron(...) must have 6 fields: minutes hours day-of-month month day-of-week year")]
    CronFieldCount,
    #[error("cron {field} field {value:?} is invalid")]
    CronField { field: &'static str, value: String },
    #[error("cron day-of-month and day-of-week: exactly one of them must be ?")]
    CronQuestionMark,
}

pub fn parse(expression: &str) -> Result<Schedule, ScheduleError> {
    let expression = expression.trim();
    if let Some(inner) = expression
        .strip_prefix("rate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_rate(inner);
    }
    if let Some(inner) = expression
        .strip_prefix("cron(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_cron(inner).map(Schedule::Cron);
    }
    Err(ScheduleError::Syntax)
}

fn parse_rate(inner: &str) -> Result<Schedule, ScheduleError> {
    let parts: Vec<&str> = inner.split_whitespace().collect();
    let [value, unit] = parts.as_slice() else {
        return Err(ScheduleError::Rate);
    };
    let value: u64 = value.parse().map_err(|_| ScheduleError::Rate)?;
    if value == 0 {
        return Err(ScheduleError::Rate);
    }
    let (unit, plural) = match *unit {
        "minute" => (RateUnit::Minute, false),
        "minutes" => (RateUnit::Minute, true),
        "hour" => (RateUnit::Hour, false),
        "hours" => (RateUnit::Hour, true),
        "day" => (RateUnit::Day, false),
        "days" => (RateUnit::Day, true),
        _ => return Err(ScheduleError::Rate),
    };
    if plural == (value == 1) {
        return Err(ScheduleError::RateUnitNumber);
    }
    Ok(Schedule::Rate { value, unit })
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

struct FieldSpec {
    name: &'static str,
    min: u64,
    max: u64,
    names: &'static [&'static str],
    question: bool,
}

fn parse_cron(inner: &str) -> Result<CronExpression, ScheduleError> {
    let fields: Vec<&str> = inner.split_whitespace().collect();
    let [minutes, hours, dom, month, dow, year] = fields.as_slice() else {
        return Err(ScheduleError::CronFieldCount);
    };
    let specs = [
        (
            *minutes,
            FieldSpec {
                name: "minutes",
                min: 0,
                max: 59,
                names: &[],
                question: false,
            },
        ),
        (
            *hours,
            FieldSpec {
                name: "hours",
                min: 0,
                max: 23,
                names: &[],
                question: false,
            },
        ),
        (
            *month,
            FieldSpec {
                name: "month",
                min: 1,
                max: 12,
                names: &MONTHS,
                question: false,
            },
        ),
        (
            *year,
            FieldSpec {
                name: "year",
                min: 1970,
                max: 2199,
                names: &[],
                question: false,
            },
        ),
    ];
    for (value, spec) in &specs {
        check_field(value, spec)?;
    }
    check_day_of_month(dom)?;
    check_day_of_week(dow)?;
    if (*dom == "?") == (*dow == "?") {
        return Err(ScheduleError::CronQuestionMark);
    }
    Ok(CronExpression {
        minutes: minutes.to_string(),
        hours: hours.to_string(),
        day_of_month: dom.to_string(),
        month: month.to_string(),
        day_of_week: dow.to_string(),
        year: year.to_string(),
    })
}

fn field_error(spec: &FieldSpec, value: &str) -> ScheduleError {
    ScheduleError::CronField {
        field: spec.name,
        value: value.to_string(),
    }
}

fn atom(spec: &FieldSpec, s: &str) -> Option<u64> {
    if let Ok(n) = s.parse::<u64>() {
        return (spec.min..=spec.max).contains(&n).then_some(n);
    }
    spec.names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(s))
        .map(|i| spec.min + i as u64)
}

fn check_field(value: &str, spec: &FieldSpec) -> Result<(), ScheduleError> {
    if value == "?" {
        return if spec.question {
            Ok(())
        } else {
            Err(field_error(spec, value))
        };
    }
    for item in value.split(',') {
        if !check_item(item, spec) {
            return Err(field_error(spec, value));
        }
    }
    Ok(())
}

fn check_item(item: &str, spec: &FieldSpec) -> bool {
    let (range, step) = match item.split_once('/') {
        Some((r, s)) => (r, Some(s)),
        None => (item, None),
    };
    if let Some(step) = step {
        match step.parse::<u64>() {
            Ok(n) if n > 0 => {}
            _ => return false,
        }
    }
    if range == "*" {
        return true;
    }
    match range.split_once('-') {
        Some((a, b)) => matches!((atom(spec, a), atom(spec, b)), (Some(a), Some(b)) if a <= b),
        None => atom(spec, range).is_some(),
    }
}

fn check_day_of_month(value: &str) -> Result<(), ScheduleError> {
    let spec = FieldSpec {
        name: "day-of-month",
        min: 1,
        max: 31,
        names: &[],
        question: true,
    };
    if value == "L" {
        return Ok(());
    }
    if let Some(day) = value.strip_suffix('W') {
        return atom(&spec, day)
            .map(|_| ())
            .ok_or_else(|| field_error(&spec, value));
    }
    check_field(value, &spec)
}

fn check_day_of_week(value: &str) -> Result<(), ScheduleError> {
    let spec = FieldSpec {
        name: "day-of-week",
        min: 1,
        max: 7,
        names: &WEEKDAYS,
        question: true,
    };
    if let Some(day) = value.strip_suffix('L')
        && !day.is_empty()
    {
        return atom(&spec, day)
            .map(|_| ())
            .ok_or_else(|| field_error(&spec, value));
    }
    if let Some((day, nth)) = value.split_once('#') {
        let nth_ok = matches!(nth.parse::<u64>(), Ok(1..=5));
        return if atom(&spec, day).is_some() && nth_ok {
            Ok(())
        } else {
            Err(field_error(&spec, value))
        };
    }
    check_field(value, &spec)
}

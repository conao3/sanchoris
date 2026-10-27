use sanchoris_config::schedule::{RateUnit, Schedule, ScheduleError, parse};

#[test]
fn parses_rate() {
    assert_eq!(
        parse("rate(5 minutes)").unwrap(),
        Schedule::Rate {
            value: 5,
            unit: RateUnit::Minute
        }
    );
    assert_eq!(
        parse("rate(1 day)").unwrap(),
        Schedule::Rate {
            value: 1,
            unit: RateUnit::Day
        }
    );
}

#[test]
fn rejects_bad_rate() {
    assert_eq!(parse("rate(1 minutes)"), Err(ScheduleError::RateUnitNumber));
    assert_eq!(parse("rate(2 hour)"), Err(ScheduleError::RateUnitNumber));
    assert_eq!(parse("rate(0 minutes)"), Err(ScheduleError::Rate));
    assert_eq!(parse("rate(5 weeks)"), Err(ScheduleError::Rate));
    assert_eq!(parse("rate(5)"), Err(ScheduleError::Rate));
}

#[test]
fn parses_cron() {
    for expr in [
        "cron(0 10 * * ? *)",
        "cron(15 12 * * ? *)",
        "cron(0 18 ? * MON-FRI *)",
        "cron(0 8 1 * ? *)",
        "cron(0/15 * * * ? *)",
        "cron(0/10 * ? * MON-FRI *)",
        "cron(0/5 8-17 ? * MON-FRI *)",
        "cron(0 9 ? * 2#1 *)",
        "cron(0 9 L * ? 2026)",
        "cron(0 9 15W * ? *)",
        "cron(0 9 ? * 6L *)",
        "cron(0,30 9 ? JAN,JUL MON *)",
    ] {
        assert!(parse(expr).is_ok(), "{expr}: {:?}", parse(expr));
    }
}

#[test]
fn rejects_bad_cron() {
    assert_eq!(
        parse("cron(0 10 * * *)"),
        Err(ScheduleError::CronFieldCount)
    );
    assert_eq!(
        parse("cron(0 10 * * * *)"),
        Err(ScheduleError::CronQuestionMark)
    );
    assert_eq!(
        parse("cron(0 10 ? * ? *)"),
        Err(ScheduleError::CronQuestionMark)
    );
    assert!(matches!(
        parse("cron(60 10 * * ? *)"),
        Err(ScheduleError::CronField {
            field: "minutes",
            ..
        })
    ));
    assert!(matches!(
        parse("cron(0 24 * * ? *)"),
        Err(ScheduleError::CronField { field: "hours", .. })
    ));
    assert!(matches!(
        parse("cron(0 10 ? FOO MON *)"),
        Err(ScheduleError::CronField { field: "month", .. })
    ));
    assert!(matches!(
        parse("cron(0 10 ? * 8 *)"),
        Err(ScheduleError::CronField {
            field: "day-of-week",
            ..
        })
    ));
    assert_eq!(parse("every 5 minutes"), Err(ScheduleError::Syntax));
}

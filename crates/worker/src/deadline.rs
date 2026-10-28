use std::future::Future;

use chrono::{DateTime, Utc};

tokio::task_local! {
    static DEADLINE: DateTime<Utc>;
}

pub async fn with_deadline<F: Future>(deadline: DateTime<Utc>, work: F) -> F::Output {
    DEADLINE.scope(deadline, work).await
}

pub fn deadline() -> Option<DateTime<Utc>> {
    DEADLINE.try_with(|deadline| *deadline).ok()
}

pub fn bounded(wanted: DateTime<Utc>) -> DateTime<Utc> {
    deadline().map_or(wanted, |deadline| wanted.min(deadline))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[tokio::test]
    async fn waits_end_at_the_invocation_deadline() {
        let now = Utc::now();
        assert_eq!(bounded(now), now);
        let limit = now + Duration::seconds(30);
        with_deadline(limit, async {
            assert_eq!(deadline(), Some(limit));
            assert_eq!(bounded(now + Duration::seconds(240)), limit);
            assert_eq!(
                bounded(now + Duration::seconds(10)),
                now + Duration::seconds(10)
            );
        })
        .await;
        assert_eq!(deadline(), None);
    }
}

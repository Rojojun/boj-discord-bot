use boj_discord_bot::{
    assignment::Problem,
    checkin::Checkins,
    recommendation::Recommendation,
    solved_ac::ApiError,
    verification::{PendingReason, SolveSource, Verifications, VerifiedStatus, VerifyError},
};
use chrono::{DateTime, Utc};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs,
    path::Path,
};

struct Source {
    identity_matches: bool,
    calls: Cell<usize>,
    responses: RefCell<VecDeque<Result<Option<bool>, ApiError>>>,
}
impl Source {
    fn new(responses: Vec<Result<Option<bool>, ApiError>>) -> Self {
        Self {
            identity_matches: true,
            calls: Cell::new(0),
            responses: RefCell::new(responses.into()),
        }
    }
}
impl SolveSource for Source {
    fn identity(&self, handle: &str) -> Result<u64, VerifyError> {
        assert_eq!(handle, "alice");
        if self.identity_matches {
            Ok(100)
        } else {
            Err(VerifyError::WrongAccount)
        }
    }
    fn status(&self, _: &Problem) -> Result<Option<bool>, ApiError> {
        self.calls.set(self.calls.get() + 1);
        self.responses
            .borrow_mut()
            .pop_front()
            .expect("unexpected extra API call")
    }
}
fn seed(data: &Path) {
    let directory = data.join("users/1/recommendations");
    fs::create_dir_all(&directory).unwrap();
    let recommendation = Recommendation {
        handle: "alice".into(),
        tier: 10,
        requested: 3,
        topic: None,
        problems: (1..=3)
            .map(|id| Problem {
                id: format!("{id}:2"),
                title: format!("problem {id}"),
                submission_url: format!("https://judge.example/{id}"),
            })
            .collect(),
    };
    fs::write(
        directory.join("2026-11-22.json"),
        serde_json::to_vec(&recommendation).unwrap(),
    )
    .unwrap();
}
fn now() -> DateTime<Utc> {
    "2026-11-22T12:00:00Z".parse().unwrap()
}

#[test]
fn verified_result_is_separate_from_manual_check_and_recovers_after_restart() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path());
    let manual = Checkins::new(data.path());
    manual.mark(1, now(), Some(2), false).unwrap();
    let source = Source::new(vec![Ok(Some(true)), Ok(Some(false)), Ok(None)]);
    let report = Verifications::new(data.path())
        .check(1, now(), &source)
        .unwrap();
    assert_eq!(
        report
            .results
            .iter()
            .map(|row| &row.status)
            .collect::<Vec<_>>(),
        [
            &VerifiedStatus::Solved,
            &VerifiedStatus::Unsolved,
            &VerifiedStatus::Pending
        ]
    );
    assert_eq!(report.results[2].reason, Some(PendingReason::UnknownStatus));
    assert_eq!(report.account_id, 100);
    assert_eq!(manual.progress(1, now()).unwrap().unwrap().count(), 1);
    assert_eq!(
        Verifications::new(data.path())
            .read(1, now())
            .unwrap()
            .unwrap()
            .results[0]
            .status,
        VerifiedStatus::Solved
    );
    assert!(
        Verifications::new(data.path())
            .read(2, now())
            .unwrap()
            .is_none()
    );
    assert!(!data.path().join("users/1/curriculum.json").exists());
}

#[test]
fn failed_lookup_preserves_historical_confirmation_and_stops_on_rate_limit() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path());
    let store = Verifications::new(data.path());
    store
        .check(
            1,
            now(),
            &Source::new(vec![Ok(Some(true)), Ok(Some(false)), Ok(None)]),
        )
        .unwrap();
    let source = Source::new(vec![Err(ApiError::RateLimited {
        retry_after_seconds: Some(30),
    })]);
    let report = store
        .check(1, now() + chrono::Duration::minutes(1), &source)
        .unwrap();
    assert_eq!(source.calls.get(), 1);
    assert!(
        report
            .results
            .iter()
            .all(|row| row.status == VerifiedStatus::Pending
                && row.reason == Some(PendingReason::RateLimited))
    );
    assert_eq!(report.results[0].last_confirmed_solved_at, Some(now()));
    assert!(report.results[1].last_confirmed_solved_at.is_none());
}

#[test]
fn wrong_identity_and_missing_recommendation_do_not_write_or_query_problems() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path());
    let store = Verifications::new(data.path());
    let mut source = Source::new(vec![]);
    source.identity_matches = false;
    assert!(matches!(
        store.check(1, now(), &source),
        Err(VerifyError::WrongAccount)
    ));
    assert_eq!(source.calls.get(), 0);
    assert!(!data.path().join("users/1/verifications").exists());
    assert!(matches!(
        store.check(2, now(), &source),
        Err(VerifyError::NoRecommendation)
    ));
    assert!(matches!(
        store.check(1, now() + chrono::Duration::days(1), &source),
        Err(VerifyError::NoRecommendation)
    ));
}

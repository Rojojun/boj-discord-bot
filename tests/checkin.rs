use boj_discord_bot::{
    assignment::Problem,
    checkin::{CheckinError, Checkins},
    pause::PauseStore,
    recommendation::Recommendation,
    settings::{LookupError, Profile, ProfileLookup, SettingsStore, SettingsUpdate},
};
use boj_discord_bot::{
    checkin::ReminderProgress,
    verification::{PendingReason, Report, ResultRow, VerifiedStatus},
};
use chrono::{DateTime, Utc};
use std::{fs, path::Path, sync::Arc};

struct Profiles;
impl ProfileLookup for Profiles {
    fn lookup(&self, handle: &str) -> Result<Profile, LookupError> {
        Ok(Profile {
            handle: handle.into(),
            tier: "10".into(),
        })
    }
}
fn time(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}
fn seed(data: &Path, user: u64, date: &str) {
    SettingsStore::new(data)
        .setup(user, "alice", 3, &Profiles)
        .unwrap();
    let directory = data.join(format!("users/{user}/recommendations"));
    fs::create_dir_all(&directory).unwrap();
    let recommendation = Recommendation {
        verified_account_id: None,
        handle: "alice".into(),
        tier: 10,
        requested: 3,
        topic: None,
        problems: (1..=3)
            .map(|id| Problem {
                id: id.to_string(),
                title: format!("problem {id}"),
                submission_url: format!("https://judge.example/{id}"),
            })
            .collect(),
    };
    fs::write(
        directory.join(format!("{date}.json")),
        serde_json::to_vec(&recommendation).unwrap(),
    )
    .unwrap();
}

fn report(statuses: &[VerifiedStatus]) -> Report {
    let now = time("2026-11-21T12:00:00Z");
    Report {
        date: now.date_naive(),
        handle: "alice".into(),
        account_id: 100,
        checked_at: now,
        results: statuses
            .iter()
            .enumerate()
            .map(|(index, status)| ResultRow {
                problem: Problem {
                    id: (index + 1).to_string(),
                    title: format!("p{}", index + 1),
                    submission_url: format!("https://judge.example/{}", index + 1),
                },
                status: status.clone(),
                reason: (*status == VerifiedStatus::Pending)
                    .then_some(PendingReason::UnknownStatus),
                last_confirmed_solved_at: None,
            })
            .collect(),
    }
}

#[test]
fn verified_completion_and_unknown_state_do_not_consume_the_daily_delivery() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path(), 1, "2026-11-21");
    let store = Checkins::new(data.path());
    let now = time("2026-11-21T12:00:00Z");
    let complete = report(&[
        VerifiedStatus::Solved,
        VerifiedStatus::Solved,
        VerifiedStatus::Solved,
    ]);
    assert!(
        store
            .claim_due_with(10, now, |_, _| Ok(ReminderProgress::from_report(&complete)))
            .unwrap()
            .is_empty()
    );
    let uncertain = report(&[
        VerifiedStatus::Solved,
        VerifiedStatus::Unsolved,
        VerifiedStatus::Pending,
    ]);
    assert!(
        store
            .claim_due_with(10, now, |_, _| Ok(ReminderProgress::from_report(
                &uncertain
            )))
            .unwrap()
            .is_empty()
    );
    let known = report(&[
        VerifiedStatus::Solved,
        VerifiedStatus::Unsolved,
        VerifiedStatus::Solved,
    ]);
    let notices = store
        .claim_due_with(10, now, |_, _| Ok(ReminderProgress::from_report(&known)))
        .unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!((notices[0].checked, notices[0].total), (2, 3));
    let pending = store
        .still_due_with(&notices[0], now, |_, _| {
            Ok(ReminderProgress::from_report(&known))
        })
        .unwrap()
        .unwrap();
    assert!(pending.verified);
    assert_eq!(pending.remaining.len(), 1);
    assert_eq!(pending.remaining[0].id, "2");
    assert!(
        Checkins::new(data.path())
            .claim_due_with(10, now, |_, _| panic!(
                "must not re-query already attempted user"
            ))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn api_mode_does_not_use_manual_done_to_hide_unresolved_problems() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path(), 1, "2026-11-21");
    let store = Checkins::new(data.path());
    let now = time("2026-11-21T12:00:00Z");
    store.mark(1, now, None, false).unwrap();
    assert!(store.claim_due(10, now).unwrap().is_empty());
    let known = report(&[
        VerifiedStatus::Solved,
        VerifiedStatus::Unsolved,
        VerifiedStatus::Solved,
    ]);
    let notices = store
        .claim_due_with(10, now, |_, _| Ok(ReminderProgress::from_report(&known)))
        .unwrap();
    assert_eq!(notices.len(), 1);
    PauseStore::new(data.path()).skip(1, now, Some(1)).unwrap();
    assert!(!store.allowed(&notices[0], now).unwrap());
    assert!(
        store
            .still_due_with(&notices[0], now, |_, _| panic!(
                "skip must suppress lookups"
            ))
            .unwrap()
            .is_none()
    );
    store.retry(&notices[0]).unwrap();
    PauseStore::new(data.path()).resume(1).unwrap();
    assert_eq!(
        store
            .claim_due_with(10, now, |_, _| Ok(ReminderProgress::from_report(&known)))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn partial_full_and_undo_checks_persist_without_affecting_verified_progress() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path(), 1, "2026-11-21");
    let now = time("2026-11-21T12:00:00Z");
    let store = Checkins::new(data.path());
    assert_eq!(store.mark(1, now, Some(1), false).unwrap().count(), 1);
    assert_eq!(store.mark(1, now, Some(1), false).unwrap().count(), 1);
    assert!(matches!(
        store.mark(1, now, Some(0), false),
        Err(CheckinError::InvalidIndex)
    ));
    assert!(matches!(
        store.mark(1, now, Some(4), false),
        Err(CheckinError::InvalidIndex)
    ));
    assert!(matches!(
        store.mark(2, now, None, false),
        Err(CheckinError::NoRecommendation)
    ));
    let restarted = Checkins::new(data.path());
    assert_eq!(restarted.progress(1, now).unwrap().unwrap().count(), 1);
    assert_eq!(restarted.mark(1, now, None, false).unwrap().count(), 3);
    assert_eq!(restarted.mark(1, now, Some(2), true).unwrap().count(), 2);
    assert_eq!(restarted.mark(1, now, None, true).unwrap().count(), 0);
    assert!(!data.path().join("users/1/curriculum.json").exists());
    assert!(!data.path().join("users/1/assignments").exists());
}

#[test]
fn reminder_claims_only_due_incomplete_active_users_and_survives_restart() {
    let data = tempfile::tempdir().unwrap();
    for user in 1..=5 {
        seed(data.path(), user, "2026-11-21");
    }
    let before = time("2026-11-21T11:59:00Z"); // 20:59 Seoul
    let now = time("2026-11-21T12:00:00Z");
    let store = Checkins::new(data.path());
    store.mark(1, now, Some(1), false).unwrap();
    store.mark(2, now, None, false).unwrap();
    PauseStore::new(data.path()).skip(3, now, Some(1)).unwrap();
    fs::remove_file(data.path().join("users/4/recommendations/2026-11-21.json")).unwrap();
    SettingsStore::new(data.path())
        .update(
            5,
            SettingsUpdate {
                reminder_time: Some("22:00"),
                ..Default::default()
            },
            &Profiles,
        )
        .unwrap();
    assert!(store.claim_due(10, before).unwrap().is_empty());
    let notices = store.claim_due(10, now).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].user, 1);
    assert_eq!((notices[0].checked, notices[0].total), (1, 3));
    let restarted = Checkins::new(data.path());
    assert!(restarted.claim_due(10, now).unwrap().is_empty());
    // Explicit nondelivery allows retry, unlike uncertain delivery/restarts.
    restarted.retry(&notices[0]).unwrap();
    let retry = restarted.claim_due(10, now).unwrap();
    assert_eq!(retry.len(), 1);
    restarted.mark(1, now, None, false).unwrap();
    assert!(restarted.still_due(&retry[0], now).unwrap().is_none());
}

#[test]
fn midnight_and_expired_pause_do_not_leak_yesterdays_check_or_reminder() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path(), 1, "2026-11-21");
    let old = time("2026-11-21T14:59:59Z");
    let next = time("2026-11-21T15:00:00Z");
    let store = Checkins::new(data.path());
    let notices = store.claim_due(10, old).unwrap();
    assert_eq!(notices.len(), 1);
    assert!(store.still_due(&notices[0], next).unwrap().is_none());
    assert!(store.progress(1, next).unwrap().is_none());
    assert!(store.claim_due(10, next).unwrap().is_empty());
    let pause = PauseStore::new(data.path());
    pause.skip(1, old, Some(1)).unwrap();
    seed(data.path(), 1, "2026-11-22");
    let next_evening = time("2026-11-22T12:00:00Z");
    assert!(store.claim_due(10, next_evening).unwrap().is_empty());
    assert!(matches!(
        store.mark(1, next_evening, None, false),
        Err(CheckinError::Paused)
    ));
    pause.resume(1).unwrap();
    assert_eq!(store.claim_due(10, next_evening).unwrap().len(), 1);
}

#[test]
fn concurrent_checkins_do_not_lose_updates_and_claims_are_exclusive() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path(), 1, "2026-11-21");
    let now = time("2026-11-21T12:00:00Z");
    let store = Arc::new(Checkins::new(data.path()));
    let workers: Vec<_> = (1..=2)
        .map(|index| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.mark(1, now, Some(index), false).unwrap().count())
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(store.progress(1, now).unwrap().unwrap().count(), 2);
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let path = data.path().to_owned();
            std::thread::spawn(move || Checkins::new(&path).claim_due(10, now).unwrap().len())
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .sum::<usize>(),
        1
    );
}

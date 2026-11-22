use std::{cell::Cell, fs, path::Path};

use boj_discord_bot::{
    assignment::{Assignment, AssignmentStore, Problem},
    reminder::{Delivery, ReminderOutcome, ReminderSender, ReminderService, SolveLookup},
    settings::{LookupError, Settings, SettingsStore},
};

struct Solves([Option<bool>; 2]);

impl SolveLookup for Solves {
    fn solved(&self, handle: &str, _: &[Problem]) -> Result<Vec<Option<bool>>, LookupError> {
        assert_eq!(handle, "assigned-handle");
        Ok(self.0.to_vec())
    }
}

struct Sender(Cell<usize>);

impl ReminderSender for Sender {
    fn send(&self, discord_id: u64, solved_count: usize, remaining: &[Problem]) -> Delivery {
        assert_eq!(
            (discord_id, solved_count, remaining[0].id.as_str()),
            (10, 1, "b")
        );
        let count = self.0.get();
        self.0.set(count + 1);
        if count == 0 {
            Delivery::Retryable
        } else {
            Delivery::Uncertain
        }
    }
}

fn seed(data: &Path) {
    let user = data.join("users/10");
    fs::create_dir_all(user.join("assignments")).unwrap();
    fs::write(
        user.join("settings.json"),
        serde_json::to_vec(&Settings {
            delivery: Default::default(),
            solved_ac_handle: "updated-handle".into(),
            tier: "Gold V".into(),
            daily_count: 2,
            reminder_time: "21:00".into(),
            goal: "tier".into(),
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(
        user.join("assignments/2026-11-20.json"),
        serde_json::to_vec(&Assignment {
            solved_ac_handle: Some("assigned-handle".into()),
            stage: None,
            requested: 2,
            problems: ["a", "b"]
                .into_iter()
                .map(|id| Problem {
                    id: id.into(),
                    title: id.into(),
                    submission_url: format!("https://example.test/{id}"),
                })
                .collect(),
        })
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn retries_confirmed_nondelivery_and_suppresses_uncertain_delivery_after_restart() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path());
    let settings = SettingsStore::new(data.path());
    let assignments = AssignmentStore::new(data.path());
    let make_service = || ReminderService::new(data.path(), &settings, &assignments);
    let service = make_service();
    let due = "2026-11-20T12:00:00Z".parse().unwrap();
    let sender = Sender(Cell::new(0));
    let partial = Solves([Some(true), Some(false)]);
    assert_eq!(
        service
            .check(10, due, &Solves([Some(true), None]), &sender)
            .unwrap(),
        ReminderOutcome::CheckPending
    );
    assert_eq!(
        service
            .check(10, due, &Solves([Some(true); 2]), &sender)
            .unwrap(),
        ReminderOutcome::Completed
    );
    assert_eq!(
        service.check(10, due, &partial, &sender).unwrap(),
        ReminderOutcome::Delivery(Delivery::Retryable)
    );
    assert_eq!(
        service.check(10, due, &partial, &sender).unwrap(),
        ReminderOutcome::Delivery(Delivery::Uncertain)
    );
    assert_eq!(
        make_service().check(10, due, &partial, &sender).unwrap(),
        ReminderOutcome::AlreadyAttempted
    );
    assert_eq!(sender.0.get(), 2);
}

use boj_discord_bot::{
    assignment::{AssignmentError, Difficulty, ProblemLookup},
    pause::{PauseStatus, PauseStore, ResumePromptSender},
};

impl ResumePromptSender for Sender {
    fn send_resume_prompt(&self, _: u64) -> Delivery {
        self.0.set(self.0.get() + 1);
        Delivery::Sent
    }
}

impl ProblemLookup for Solves {
    fn candidates(
        &self,
        _: &str,
        _: &str,
        _: Difficulty,
        _: &[Problem],
        _: usize,
    ) -> Result<Vec<Problem>, LookupError> {
        panic!("paused users and saved assignments require no problem lookup")
    }
}

#[test]
fn pauses_assignment_and_reminders_until_explicit_resume() {
    let data = tempfile::tempdir().unwrap();
    seed(data.path());
    let settings = SettingsStore::new(data.path());
    let assignments = AssignmentStore::new(data.path());
    let pause = PauseStore::new(data.path());
    let reminders = ReminderService::new(data.path(), &settings, &assignments);
    let today = "2026-11-20T12:00:00Z".parse().unwrap();
    let expiry = "2026-11-26T15:00:00Z".parse().unwrap();
    let solves = Solves([Some(true), Some(false)]);
    let sender = Sender(Cell::new(0));

    let status = pause.skip(10, today, Some(7)).unwrap();
    assert_eq!(
        status,
        PauseStatus::SkippedUntil("2026-11-27".parse().unwrap())
    );
    assert_eq!(pause.skip(10, today, None).unwrap(), status);
    assert!(matches!(
        assignments.today(10, today, &settings, &solves),
        Err(AssignmentError::Paused {
            existing: Some(_),
            ..
        })
    ));
    assert_eq!(
        reminders.check(10, today, &solves, &sender).unwrap(),
        ReminderOutcome::Paused
    );
    assert_eq!(pause.prompt_if_expired(10, today, &sender).unwrap(), None);
    assert_eq!(
        pause.prompt_if_expired(10, expiry, &sender).unwrap(),
        Some(Delivery::Sent)
    );
    let restarted = PauseStore::new(data.path());
    assert_eq!(
        restarted.prompt_if_expired(10, expiry, &sender).unwrap(),
        None
    );
    assert_eq!(
        reminders.check(10, expiry, &solves, &sender).unwrap(),
        ReminderOutcome::Paused
    );
    restarted.resume(10).unwrap();
    assert_eq!(
        reminders.check(10, today, &solves, &sender).unwrap(),
        ReminderOutcome::Delivery(Delivery::Uncertain)
    );
}

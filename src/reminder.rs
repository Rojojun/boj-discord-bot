use crate::assignment::{AssignmentStore, Problem, seoul_time};
use crate::curriculum::CurriculumStore;
use crate::pause::{self, PauseStatus};
use crate::settings::{LookupError, SettingsStore};
use chrono::{DateTime, NaiveTime, Utc};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub trait SolveLookup {
    /// Return each problem's known solve status in input order.
    fn solved(&self, handle: &str, problems: &[Problem]) -> Result<Vec<Option<bool>>, LookupError>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    Retryable,
    Uncertain,
}

pub trait ReminderSender {
    fn send(&self, discord_id: u64, solved_count: usize, remaining: &[Problem]) -> Delivery;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReminderOutcome {
    NotDue,
    Paused,
    NoAssignment,
    CheckPending,
    Completed,
    AlreadyAttempted,
    Delivery(Delivery),
}

pub struct ReminderService<'a> {
    data_dir: PathBuf,
    settings: &'a SettingsStore,
    assignments: &'a AssignmentStore,
    curriculum: CurriculumStore,
}

impl<'a> ReminderService<'a> {
    pub fn new(
        data_dir: &Path,
        settings: &'a SettingsStore,
        assignments: &'a AssignmentStore,
    ) -> Self {
        Self {
            data_dir: data_dir.into(),
            settings,
            assignments,
            curriculum: CurriculumStore::new(data_dir),
        }
    }

    pub fn check(
        &self,
        discord_id: u64,
        now: DateTime<Utc>,
        lookup: &impl SolveLookup,
        sender: &impl ReminderSender,
    ) -> io::Result<ReminderOutcome> {
        let local = seoul_time(now);
        let date = local.date_naive();
        let users = self.data_dir.join("users");
        if pause::status(&users, discord_id, date)? != PauseStatus::Active {
            return Ok(ReminderOutcome::Paused);
        }
        let Some(settings) = self.settings.read(discord_id)? else {
            return Ok(ReminderOutcome::NoAssignment);
        };
        let reminder_time = NaiveTime::parse_from_str(&settings.reminder_time, "%H:%M")
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if local.time() < reminder_time {
            return Ok(ReminderOutcome::NotDue);
        }

        let directory = self
            .data_dir
            .join("users")
            .join(discord_id.to_string())
            .join("reminders");
        let marker = directory.join(format!("{date}.json"));
        let attempted = marker.try_exists()?;
        let Some(assignment) = self.assignments.read(discord_id, date)? else {
            return Ok(ReminderOutcome::NoAssignment);
        };
        if attempted && assignment.stage.is_none() {
            return Ok(ReminderOutcome::AlreadyAttempted);
        }
        let Some(handle) = assignment.solved_ac_handle else {
            return Ok(ReminderOutcome::CheckPending);
        };
        let Ok(statuses) = lookup.solved(&handle, &assignment.problems) else {
            return Ok(ReminderOutcome::CheckPending);
        };
        if statuses.len() != assignment.problems.len() || statuses.iter().any(Option::is_none) {
            return Ok(ReminderOutcome::CheckPending);
        }
        if let Some(stage) = &assignment.stage {
            let solved: Vec<_> = assignment
                .problems
                .iter()
                .zip(&statuses)
                .filter_map(|(problem, status)| {
                    (*status == Some(true)).then_some(problem.id.as_str())
                })
                .collect();
            self.curriculum.record_solved(discord_id, stage, &solved)?;
        }
        if attempted {
            return Ok(ReminderOutcome::AlreadyAttempted);
        }
        let total = assignment.problems.len();
        let remaining: Vec<_> = assignment
            .problems
            .into_iter()
            .zip(statuses)
            .filter_map(|(problem, solved)| (solved == Some(false)).then_some(problem))
            .collect();
        if remaining.is_empty() {
            return Ok(ReminderOutcome::Completed);
        }

        if pause::status(&users, discord_id, date)? != PauseStatus::Active {
            return Ok(ReminderOutcome::Paused);
        }
        fs::create_dir_all(&directory)?;
        let mut claim = tempfile::NamedTempFile::new_in(directory)?;
        claim.write_all(b"{\"attempted\":true}\n")?;
        claim.as_file().sync_all()?;
        match claim.persist_noclobber(&marker) {
            Ok(_) => {}
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                return Ok(ReminderOutcome::AlreadyAttempted);
            }
            Err(error) => return Err(error.error),
        }
        let delivery = sender.send(discord_id, total - remaining.len(), &remaining);
        if delivery == Delivery::Retryable {
            fs::remove_file(marker)?;
        }
        Ok(ReminderOutcome::Delivery(delivery))
    }
}

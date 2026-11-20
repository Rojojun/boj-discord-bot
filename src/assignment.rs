use crate::curriculum::{CurriculumStore, Plan, Stage, StageKey};
use crate::pause::{self, PauseStatus};
use crate::problem_help::HelpError;
use crate::settings::{LookupError, SettingsStore};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub id: String,
    pub title: String,
    pub submission_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Same,
    OneLower,
    OneHigher,
}

pub trait ProblemLookup {
    /// Return up to limit unsolved, submittable problems at the requested difficulty.
    /// Skip excluded IDs. A difficulty outside the provider's range yields no candidates.
    fn candidates(
        &self,
        handle: &str,
        tier: &str,
        difficulty: Difficulty,
        excluded: &[Problem],
        limit: usize,
    ) -> Result<Vec<Problem>, LookupError>;

    /// Sources that support curricula must apply the stage's problem constraints.
    fn stage_candidates(
        &self,
        _handle: &str,
        _tier: &str,
        _stage: Stage,
        _difficulty: Difficulty,
        _excluded: &[Problem],
        _limit: usize,
    ) -> Result<Vec<Problem>, LookupError> {
        Err(LookupError::Unavailable)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    #[serde(default)]
    pub solved_ac_handle: Option<String>,
    #[serde(default)]
    pub stage: Option<StageKey>,
    pub requested: u8,
    pub problems: Vec<Problem>,
}

impl Assignment {
    pub fn shortfall(&self) -> usize {
        usize::from(self.requested).saturating_sub(self.problems.len())
    }
}

#[derive(Debug)]
pub enum AssignmentError {
    SetupRequired,
    CurriculumCompleted {
        track: String,
    },
    Paused {
        status: PauseStatus,
        existing: Option<Assignment>,
    },
    Lookup(LookupError),
    Storage(io::Error),
}

pub struct AssignmentStore {
    users: PathBuf,
    curriculum: CurriculumStore,
    writes: Mutex<()>,
}

impl AssignmentStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            users: data_dir.join("users"),
            curriculum: CurriculumStore::new(data_dir),
            writes: Mutex::new(()),
        }
    }

    pub fn read(&self, discord_id: u64, date: NaiveDate) -> io::Result<Option<Assignment>> {
        read(&self.directory(discord_id).join(format!("{date}.json")))
    }

    fn directory(&self, discord_id: u64) -> PathBuf {
        self.users.join(discord_id.to_string()).join("assignments")
    }

    pub(crate) fn replace_problem(
        &self,
        user: u64,
        date: NaiveDate,
        original_id: &str,
        replacement: &Problem,
        validate: impl FnOnce(&Assignment, &Problem) -> Result<(), HelpError>,
    ) -> Result<(), HelpError> {
        let _write = self
            .writes
            .lock()
            .map_err(|error| HelpError::Storage(io::Error::other(error.to_string())))?;
        if pause::status(&self.users, user, date).map_err(HelpError::Storage)?
            != PauseStatus::Active
        {
            return Err(HelpError::Paused);
        }
        let mut assignment = self
            .read(user, date)
            .map_err(HelpError::Storage)?
            .ok_or(HelpError::StaleSelection)?;
        let index = assignment
            .problems
            .iter()
            .position(|problem| problem.id == original_id)
            .ok_or(HelpError::StaleSelection)?;
        if assignment
            .problems
            .iter()
            .any(|problem| problem.id == replacement.id)
        {
            return Err(HelpError::StaleSelection);
        }
        validate(&assignment, &assignment.problems[index])?;
        if pause::status(&self.users, user, date).map_err(HelpError::Storage)?
            != PauseStatus::Active
        {
            return Err(HelpError::Paused);
        }
        assignment.problems[index] = replacement.clone();
        let directory = self.directory(user);
        write(
            &directory,
            &directory.join(format!("{date}.json")),
            &assignment,
        )
        .map_err(HelpError::Storage)
    }

    pub fn today(
        &self,
        discord_id: u64,
        now: DateTime<Utc>,
        settings: &SettingsStore,
        lookup: &impl ProblemLookup,
    ) -> Result<Assignment, AssignmentError> {
        let date = seoul_time(now).date_naive();
        let directory = self.directory(discord_id);
        let path = directory.join(format!("{date}.json"));
        let existing = read(&path).map_err(AssignmentError::Storage)?;
        let status =
            pause::status(&self.users, discord_id, date).map_err(AssignmentError::Storage)?;
        if status != PauseStatus::Active {
            return Err(AssignmentError::Paused { status, existing });
        }
        if let Some(saved) = existing {
            return Ok(saved);
        }

        let settings = settings
            .read(discord_id)
            .map_err(AssignmentError::Storage)?
            .ok_or(AssignmentError::SetupRequired)?;
        let (stage_key, stage) = match self
            .curriculum
            .plan(discord_id)
            .map_err(AssignmentError::Storage)?
        {
            Plan::General => (None, None),
            Plan::Learning { key, stage } => (Some(key), Some(stage)),
            Plan::Completed { track } => {
                return Err(AssignmentError::CurriculumCompleted { track });
            }
        };
        let mut assignment = Assignment {
            stage: stage_key,
            solved_ac_handle: Some(settings.solved_ac_handle.clone()),
            requested: settings.daily_count,
            problems: Vec::with_capacity(usize::from(settings.daily_count)),
        };
        for difficulty in [
            Difficulty::Same,
            Difficulty::OneLower,
            Difficulty::OneHigher,
        ] {
            let limit = assignment.shortfall();
            if limit == 0 {
                break;
            }
            let candidates = match stage {
                Some(stage) => lookup.stage_candidates(
                    &settings.solved_ac_handle,
                    &settings.tier,
                    stage,
                    difficulty,
                    &assignment.problems,
                    limit,
                ),
                None => lookup.candidates(
                    &settings.solved_ac_handle,
                    &settings.tier,
                    difficulty,
                    &assignment.problems,
                    limit,
                ),
            }
            .map_err(AssignmentError::Lookup)?;
            for problem in candidates {
                if !assignment
                    .problems
                    .iter()
                    .any(|saved| saved.id == problem.id)
                {
                    assignment.problems.push(problem);
                }
                if assignment.shortfall() == 0 {
                    break;
                }
            }
        }

        let _write = self
            .writes
            .lock()
            .map_err(|error| AssignmentError::Storage(io::Error::other(error.to_string())))?;
        let existing = read(&path).map_err(AssignmentError::Storage)?;
        let status =
            pause::status(&self.users, discord_id, date).map_err(AssignmentError::Storage)?;
        if status != PauseStatus::Active {
            return Err(AssignmentError::Paused { status, existing });
        }
        if let Some(saved) = existing {
            return Ok(saved);
        }
        if !assignment.problems.is_empty() {
            write(&directory, &path, &assignment).map_err(AssignmentError::Storage)?;
        }
        Ok(assignment)
    }
}

pub(crate) fn seoul_time(now: DateTime<Utc>) -> DateTime<FixedOffset> {
    now.with_timezone(&FixedOffset::east_opt(9 * 3600).expect("valid Seoul offset"))
}

fn read(path: &Path) -> io::Result<Option<Assignment>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    serde_json::from_reader(file)
        .map(Some)
        .map_err(io::Error::other)
}

fn write(directory: &Path, path: &Path, assignment: &Assignment) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer_pretty(&mut file, assignment).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

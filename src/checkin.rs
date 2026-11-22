//! Self-reported progress, deliberately separate from verified solve records.
use crate::{
    assignment::Problem,
    assignment::seoul_time,
    pause::{self, PauseStatus},
    recommendation::Recommendation,
    settings::SettingsStore,
    verification::{Report, VerifiedStatus},
};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Default, Serialize, Deserialize)]
struct Checked {
    ids: BTreeSet<String>,
}

pub struct Progress {
    pub recommendation: Recommendation,
    pub checked: BTreeSet<String>,
}
impl Progress {
    pub fn count(&self) -> usize {
        self.recommendation
            .problems
            .iter()
            .filter(|p| self.checked.contains(&p.id))
            .count()
    }
    pub fn total(&self) -> usize {
        self.recommendation.problems.len()
    }
}

#[derive(Debug)]
pub enum CheckinError {
    NoRecommendation,
    Paused,
    InvalidIndex,
    Storage(io::Error),
}
impl From<io::Error> for CheckinError {
    fn from(error: io::Error) -> Self {
        Self::Storage(error)
    }
}

pub struct Notice {
    pub user: u64,
    pub date: NaiveDate,
    pub checked: usize,
    pub total: usize,
    channel: u64,
}

pub struct ReminderProgress {
    pub checked: usize,
    pub total: usize,
    pub remaining: Vec<Problem>,
    pub verified: bool,
}
impl ReminderProgress {
    pub fn manual(progress: &Progress) -> Option<Self> {
        let remaining: Vec<_> = progress
            .recommendation
            .problems
            .iter()
            .filter(|p| !progress.checked.contains(&p.id))
            .cloned()
            .collect();
        (!remaining.is_empty()).then(|| Self {
            checked: progress.count(),
            total: progress.total(),
            remaining,
            verified: false,
        })
    }
    pub fn from_report(report: &Report) -> Option<Self> {
        if report
            .results
            .iter()
            .any(|row| row.status == VerifiedStatus::Pending)
        {
            return None;
        }
        let remaining: Vec<_> = report
            .results
            .iter()
            .filter(|row| row.status == VerifiedStatus::Unsolved)
            .map(|row| row.problem.clone())
            .collect();
        (!remaining.is_empty()).then(|| Self {
            checked: report.results.len() - remaining.len(),
            total: report.results.len(),
            remaining,
            verified: true,
        })
    }
}

pub struct Checkins {
    data: PathBuf,
    writes: Mutex<()>,
}
impl Checkins {
    pub fn new(data: &Path) -> Self {
        Self {
            data: data.into(),
            writes: Mutex::new(()),
        }
    }
    fn user_dir(&self, user: u64) -> PathBuf {
        self.data.join("users").join(user.to_string())
    }
    fn checked_path(&self, user: u64, date: NaiveDate) -> PathBuf {
        self.user_dir(user)
            .join("checkins")
            .join(format!("{date}.json"))
    }

    pub fn progress(&self, user: u64, now: DateTime<Utc>) -> io::Result<Option<Progress>> {
        self.progress_on(user, seoul_time(now).date_naive())
    }
    fn progress_on(&self, user: u64, date: NaiveDate) -> io::Result<Option<Progress>> {
        let Some(recommendation) = read::<Recommendation>(
            &self
                .user_dir(user)
                .join("recommendations")
                .join(format!("{date}.json")),
        )?
        else {
            return Ok(None);
        };
        let checked = read::<Checked>(&self.checked_path(user, date))?.unwrap_or_default();
        Ok(Some(Progress {
            recommendation,
            checked: checked.ids,
        }))
    }

    /// None selects all today's problems; indices are the 1-based /today order.
    pub fn mark(
        &self,
        user: u64,
        now: DateTime<Utc>,
        index: Option<usize>,
        undo: bool,
    ) -> Result<Progress, CheckinError> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| io::Error::other("checkin lock poisoned"))?;
        let date = seoul_time(now).date_naive();
        if pause::status(&self.data.join("users"), user, date)? != PauseStatus::Active {
            return Err(CheckinError::Paused);
        }
        let mut progress = self
            .progress_on(user, date)?
            .ok_or(CheckinError::NoRecommendation)?;
        let selected: Vec<_> = if let Some(index) = index {
            vec![
                progress
                    .recommendation
                    .problems
                    .get(index.checked_sub(1).ok_or(CheckinError::InvalidIndex)?)
                    .ok_or(CheckinError::InvalidIndex)?,
            ]
        } else {
            progress.recommendation.problems.iter().collect()
        };
        if selected.is_empty() {
            return Err(CheckinError::NoRecommendation);
        }
        for problem in selected {
            if undo {
                progress.checked.remove(&problem.id);
            } else {
                progress.checked.insert(problem.id.clone());
            }
        }
        let path = self.checked_path(user, date);
        let directory = path.parent().expect("checkin file has parent");
        fs::create_dir_all(directory)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(
            &mut temporary,
            &Checked {
                ids: progress.checked.clone(),
            },
        )
        .map_err(io::Error::other)?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(progress)
    }

    fn eligible(&self, user: u64, now: DateTime<Utc>) -> io::Result<Option<Progress>> {
        let local = seoul_time(now);
        if pause::status(&self.data.join("users"), user, local.date_naive())? != PauseStatus::Active
        {
            return Ok(None);
        }
        let Some(settings) = SettingsStore::new(&self.data).read(user)? else {
            return Ok(None);
        };
        let time = NaiveTime::parse_from_str(&settings.reminder_time, "%H:%M")
            .map_err(io::Error::other)?;
        if local.time() < time {
            return Ok(None);
        }
        Ok(self
            .progress(user, now)?
            .filter(|progress| progress.total() > 0))
    }

    fn marker(&self, notice: &Notice) -> PathBuf {
        self.user_dir(notice.user)
            .join("checkin-reminders")
            .join(notice.channel.to_string())
            .join(format!("{}.json", notice.date))
    }

    /// Persist before network send. Ambiguous delivery is not retried that day.
    pub fn claim_due(&self, channel: u64, now: DateTime<Utc>) -> io::Result<Vec<Notice>> {
        self.claim_due_with(channel, now, |_, progress| {
            Ok(ReminderProgress::manual(&progress))
        })
    }

    pub fn claim_due_with(
        &self,
        channel: u64,
        now: DateTime<Utc>,
        mut resolve: impl FnMut(u64, Progress) -> io::Result<Option<ReminderProgress>>,
    ) -> io::Result<Vec<Notice>> {
        let users = match fs::read_dir(self.data.join("users")) {
            Ok(users) => users,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut notices = Vec::new();
        for entry in users {
            let Ok(entry) = entry else {
                continue;
            };
            if !matches!(entry.file_type(), Ok(kind) if kind.is_dir()) {
                continue;
            }
            let Some(user) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u64>().ok())
            else {
                continue;
            };
            let outcome = (|| -> io::Result<Option<Notice>> {
                let mut notice = Notice {
                    user,
                    date: seoul_time(now).date_naive(),
                    checked: 0,
                    total: 0,
                    channel,
                };
                let marker = self.marker(&notice);
                if marker.try_exists()? {
                    return Ok(None);
                }
                let Some(progress) = self.eligible(user, now)? else {
                    return Ok(None);
                };
                let Some(progress) = resolve(user, progress)? else {
                    return Ok(None);
                };
                notice.checked = progress.checked;
                notice.total = progress.total;
                let directory = marker.parent().expect("marker has parent");
                fs::create_dir_all(directory)?;
                let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
                temporary.write_all(b"{\"attempted\":true}\n")?;
                temporary.as_file().sync_all()?;
                match temporary.persist_noclobber(marker) {
                    Ok(_) => Ok(Some(notice)),
                    Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(None),
                    Err(error) => Err(error.error),
                }
            })();
            match outcome {
                Ok(Some(notice)) => notices.push(notice),
                Ok(None) => {}
                Err(_) => {
                    eprintln!(
                        "사용자 {user}의 알림 조회를 보류했습니다. 인증·API·저장 상태를 확인하세요."
                    )
                }
            }
        }
        Ok(notices)
    }

    pub fn still_due(
        &self,
        notice: &Notice,
        now: DateTime<Utc>,
    ) -> io::Result<Option<(usize, usize)>> {
        Ok(self
            .still_due_with(notice, now, |_, progress| {
                Ok(ReminderProgress::manual(&progress))
            })?
            .map(|progress| (progress.checked, progress.total)))
    }

    pub fn still_due_with(
        &self,
        notice: &Notice,
        now: DateTime<Utc>,
        mut resolve: impl FnMut(u64, Progress) -> io::Result<Option<ReminderProgress>>,
    ) -> io::Result<Option<ReminderProgress>> {
        if notice.date != seoul_time(now).date_naive() {
            return Ok(None);
        }
        match self.eligible(notice.user, now)? {
            Some(progress) => resolve(notice.user, progress),
            None => Ok(None),
        }
    }

    pub fn allowed(&self, notice: &Notice, now: DateTime<Utc>) -> io::Result<bool> {
        Ok(notice.date == seoul_time(now).date_naive()
            && self.eligible(notice.user, now)?.is_some())
    }
    /// Only use after an explicit response proving Discord rejected the send.
    pub fn retry(&self, notice: &Notice) -> io::Result<()> {
        match fs::remove_file(self.marker(notice)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    match File::open(path) {
        Ok(file) => serde_json::from_reader(file)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

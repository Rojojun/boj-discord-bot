use crate::{
    assignment::{Problem, seoul_time},
    recommendation::Recommendation,
    solved_ac::{ApiError, SolvedAc, Tokens},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Debug)]
pub enum VerifyError {
    NoRecommendation,
    WrongAccount,
    Api(ApiError),
    Storage(io::Error),
}
impl From<io::Error> for VerifyError {
    fn from(error: io::Error) -> Self {
        Self::Storage(error)
    }
}

pub trait SolveSource {
    /// Resolve the saved handle and token owner to the same stable account ID.
    fn identity(&self, handle: &str) -> Result<u64, VerifyError>;
    fn status(&self, problem: &Problem) -> Result<Option<bool>, ApiError>;
}

pub struct OAuthSolves<'a> {
    pub api: &'a SolvedAc,
    pub tokens: Tokens,
    pub account_id: u64,
}
impl SolveSource for OAuthSolves<'_> {
    fn identity(&self, handle: &str) -> Result<u64, VerifyError> {
        let expected = self.api.user_account(handle).map_err(VerifyError::Api)?;
        let actual = self.api.account(&self.tokens).map_err(VerifyError::Api)?;
        if expected.id != actual.id || actual.id != self.account_id {
            return Err(VerifyError::WrongAccount);
        }
        Ok(actual.id)
    }
    fn status(&self, problem: &Problem) -> Result<Option<bool>, ApiError> {
        let (id, sub) = problem.id.split_once(':').unwrap_or((&problem.id, ""));
        let id = id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or(ApiError::InvalidResponse)?;
        self.api.solve_status(&self.tokens, id, sub)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerifiedStatus {
    Solved,
    Unsolved,
    Pending,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingReason {
    UnknownStatus,
    Permission,
    RateLimited,
    Unavailable,
    NotFound,
    InvalidResponse,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultRow {
    pub problem: Problem,
    pub status: VerifiedStatus,
    pub reason: Option<PendingReason>,
    pub last_confirmed_solved_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub date: NaiveDate,
    pub handle: String,
    pub account_id: u64,
    pub checked_at: DateTime<Utc>,
    pub results: Vec<ResultRow>,
}

pub struct Verifications {
    data: PathBuf,
    writes: Mutex<()>,
}
impl Verifications {
    pub fn new(data: &Path) -> Self {
        Self {
            data: data.into(),
            writes: Mutex::new(()),
        }
    }
    fn path(&self, user: u64, date: NaiveDate) -> PathBuf {
        self.data
            .join("users")
            .join(user.to_string())
            .join("verifications")
            .join(format!("{date}.json"))
    }
    pub fn read(&self, user: u64, now: DateTime<Utc>) -> io::Result<Option<Report>> {
        read(&self.path(user, seoul_time(now).date_naive()))
    }
    pub fn check(
        &self,
        user: u64,
        now: DateTime<Utc>,
        source: &impl SolveSource,
    ) -> Result<Report, VerifyError> {
        let _guard = self
            .writes
            .lock()
            .map_err(|_| io::Error::other("verification lock poisoned"))?;
        let date = seoul_time(now).date_naive();
        let recommendation: Recommendation = read(
            &self
                .data
                .join("users")
                .join(user.to_string())
                .join("recommendations")
                .join(format!("{date}.json")),
        )?
        .ok_or(VerifyError::NoRecommendation)?;
        let account_id = source.identity(&recommendation.handle)?;
        let previous = self
            .read(user, now)?
            .filter(|report| report.account_id == account_id);
        let mut stopped: Option<PendingReason> = None;
        let mut results = Vec::new();
        for problem in recommendation.problems {
            let result = if let Some(reason) = &stopped {
                Err(reason.clone())
            } else {
                source.status(&problem).map_err(|error| {
                    let reason = match error {
                        ApiError::Permission => PendingReason::Permission,
                        ApiError::RateLimited { .. } => PendingReason::RateLimited,
                        ApiError::NotFound => PendingReason::NotFound,
                        ApiError::InvalidResponse => PendingReason::InvalidResponse,
                        _ => PendingReason::Unavailable,
                    };
                    if matches!(
                        reason,
                        PendingReason::Permission
                            | PendingReason::RateLimited
                            | PendingReason::Unavailable
                    ) {
                        stopped = Some(reason.clone());
                    }
                    reason
                })
            };
            let last = previous
                .as_ref()
                .and_then(|report| {
                    report
                        .results
                        .iter()
                        .find(|row| row.problem.id == problem.id)
                })
                .and_then(|row| row.last_confirmed_solved_at);
            let (status, reason, last_confirmed_solved_at) = match result {
                Ok(Some(true)) => (VerifiedStatus::Solved, None, Some(now)),
                Ok(Some(false)) => (VerifiedStatus::Unsolved, None, last),
                Ok(None) => (
                    VerifiedStatus::Pending,
                    Some(PendingReason::UnknownStatus),
                    last,
                ),
                Err(reason) => (VerifiedStatus::Pending, Some(reason), last),
            };
            results.push(ResultRow {
                problem,
                status,
                reason,
                last_confirmed_solved_at,
            });
        }
        let report = Report {
            date,
            handle: recommendation.handle,
            account_id,
            checked_at: now,
            results,
        };
        let path = self.path(user, date);
        let directory = path.parent().expect("report has parent");
        fs::create_dir_all(directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(&mut file, &report).map_err(io::Error::other)?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(report)
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

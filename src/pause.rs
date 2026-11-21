use crate::{assignment::seoul_time, reminder::Delivery};
use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseStatus {
    Active,
    SkippedUntil(NaiveDate),
    AwaitingResume { expired_on: NaiveDate },
}

#[derive(Serialize, Deserialize)]
struct Pause {
    expires_on: NaiveDate,
    prompt_attempted: bool,
}

#[derive(Debug)]
pub enum PauseError {
    InvalidDays,
    Storage(io::Error),
}

pub trait ResumePromptSender {
    fn send_resume_prompt(&self, discord_id: u64) -> Delivery;
}

pub struct PauseStore {
    users: PathBuf,
    writes: Mutex<()>,
}

impl PauseStore {
    pub fn current(&self, user: u64, now: DateTime<Utc>) -> io::Result<PauseStatus> {
        status(&self.users, user, seoul_time(now).date_naive())
    }

    pub fn new(data_dir: &Path) -> Self {
        Self {
            users: data_dir.join("users"),
            writes: Mutex::new(()),
        }
    }

    pub fn skip(
        &self,
        discord_id: u64,
        now: DateTime<Utc>,
        days: Option<u8>,
    ) -> Result<PauseStatus, PauseError> {
        let days = days.unwrap_or(1);
        if !(1..=7).contains(&days) {
            return Err(PauseError::InvalidDays);
        }
        let today = seoul_time(now).date_naive();
        let _write = self
            .writes
            .lock()
            .map_err(|error| PauseError::Storage(io::Error::other(error.to_string())))?;
        let current = status(&self.users, discord_id, today).map_err(PauseError::Storage)?;
        if matches!(current, PauseStatus::SkippedUntil(_)) {
            return Ok(current);
        }
        let expires_on = today
            .checked_add_days(Days::new(days.into()))
            .ok_or(PauseError::InvalidDays)?;
        self.write(
            discord_id,
            &Pause {
                expires_on,
                prompt_attempted: false,
            },
        )
        .map_err(PauseError::Storage)?;
        Ok(PauseStatus::SkippedUntil(expires_on))
    }

    pub fn resume(&self, discord_id: u64) -> io::Result<()> {
        let _write = self
            .writes
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        match fs::remove_file(path(&self.users, discord_id)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    pub fn prompt_if_expired(
        &self,
        discord_id: u64,
        now: DateTime<Utc>,
        sender: &impl ResumePromptSender,
    ) -> io::Result<Option<Delivery>> {
        let _write = self
            .writes
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let Some(mut pause) = read(&self.users, discord_id)? else {
            return Ok(None);
        };
        if pause.expires_on > seoul_time(now).date_naive() || pause.prompt_attempted {
            return Ok(None);
        }
        pause.prompt_attempted = true;
        self.write(discord_id, &pause)?;
        let delivery = sender.send_resume_prompt(discord_id);
        if delivery == Delivery::Retryable {
            pause.prompt_attempted = false;
            self.write(discord_id, &pause)?;
        }
        Ok(Some(delivery))
    }

    fn write(&self, discord_id: u64, pause: &Pause) -> io::Result<()> {
        let destination = path(&self.users, discord_id);
        let directory = self.users.join(discord_id.to_string());
        fs::create_dir_all(&directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(&mut file, pause).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(destination).map_err(|error| error.error)?;
        Ok(())
    }
}

pub(crate) fn status(users: &Path, discord_id: u64, today: NaiveDate) -> io::Result<PauseStatus> {
    Ok(match read(users, discord_id)? {
        None => PauseStatus::Active,
        Some(pause) if today < pause.expires_on => PauseStatus::SkippedUntil(pause.expires_on),
        Some(pause) => PauseStatus::AwaitingResume {
            expired_on: pause.expires_on,
        },
    })
}

fn path(users: &Path, discord_id: u64) -> PathBuf {
    users.join(discord_id.to_string()).join("pause.json")
}

fn read(users: &Path, discord_id: u64) -> io::Result<Option<Pause>> {
    let file = match File::open(path(users, discord_id)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    serde_json::from_reader(file)
        .map(Some)
        .map_err(io::Error::other)
}

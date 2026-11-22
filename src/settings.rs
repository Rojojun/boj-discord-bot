use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    #[default]
    Channel,
    Dm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub delivery: DeliveryMode,
    pub solved_ac_handle: String,
    pub tier: String,
    pub daily_count: u8,
    pub reminder_time: String,
    pub goal: String,
}

#[derive(Debug, Default)]
pub struct SettingsUpdate<'a> {
    pub delivery: Option<DeliveryMode>,
    pub handle: Option<&'a str>,
    pub daily_count: Option<u8>,
    pub reminder_time: Option<&'a str>,
    pub goal: Option<&'a str>,
}

pub struct Profile {
    pub handle: String,
    pub tier: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LookupError {
    UnknownHandle,
    Unavailable,
}

pub trait ProfileLookup {
    fn lookup(&self, handle: &str) -> Result<Profile, LookupError>;
}

#[derive(Debug)]
pub enum SettingsError {
    InvalidCount,
    InvalidHandle,
    InvalidReminderTime,
    InvalidGoal,
    SetupRequired,
    Lookup(LookupError),
    Storage(io::Error),
}

pub struct SettingsStore {
    users: PathBuf,
    writes: Mutex<()>,
}

impl SettingsStore {
    pub fn users(&self) -> io::Result<Vec<u64>> {
        let entries = match fs::read_dir(&self.users) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut users = Vec::new();
        for entry in entries {
            let entry = entry?;
            if let Some(user) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u64>().ok())
                && self.read(user)?.is_some()
            {
                users.push(user);
            }
        }
        Ok(users)
    }
    pub fn new(data_dir: &Path) -> Self {
        Self {
            users: data_dir.join("users"),
            writes: Mutex::new(()),
        }
    }

    fn path(&self, discord_id: u64) -> PathBuf {
        self.users
            .join(discord_id.to_string())
            .join("settings.json")
    }

    pub fn read(&self, discord_id: u64) -> io::Result<Option<Settings>> {
        let file = match File::open(self.path(discord_id)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        serde_json::from_reader(file)
            .map(Some)
            .map_err(io::Error::other)
    }

    pub fn setup(
        &self,
        discord_id: u64,
        handle: &str,
        daily_count: u8,
        profiles: &impl ProfileLookup,
    ) -> Result<Settings, SettingsError> {
        self.save(
            discord_id,
            SettingsUpdate {
                handle: Some(handle),
                daily_count: Some(daily_count),
                ..Default::default()
            },
            profiles,
            true,
        )
    }

    pub fn update(
        &self,
        discord_id: u64,
        update: SettingsUpdate<'_>,
        profiles: &impl ProfileLookup,
    ) -> Result<Settings, SettingsError> {
        self.save(discord_id, update, profiles, false)
    }

    fn save(
        &self,
        discord_id: u64,
        update: SettingsUpdate<'_>,
        profiles: &impl ProfileLookup,
        setup: bool,
    ) -> Result<Settings, SettingsError> {
        if update
            .daily_count
            .is_some_and(|count| !(1..=10).contains(&count))
        {
            return Err(SettingsError::InvalidCount);
        }
        if update.reminder_time.is_some_and(|time| !valid_time(time)) {
            return Err(SettingsError::InvalidReminderTime);
        }
        if update.goal.is_some_and(|goal| goal.trim().is_empty()) {
            return Err(SettingsError::InvalidGoal);
        }
        let profile = update
            .handle
            .map(|handle| {
                let handle = handle.trim();
                if handle.is_empty() {
                    return Err(SettingsError::InvalidHandle);
                }
                profiles.lookup(handle).map_err(SettingsError::Lookup)
            })
            .transpose()?;

        let _write = self
            .writes
            .lock()
            .map_err(|error| SettingsError::Storage(io::Error::other(error.to_string())))?;
        let mut settings = match self.read(discord_id).map_err(SettingsError::Storage)? {
            Some(settings) => settings,
            None if setup => Settings {
                delivery: DeliveryMode::Channel,
                solved_ac_handle: String::new(),
                tier: String::new(),
                daily_count: 1,
                reminder_time: "21:00".into(),
                goal: "tier".into(),
            },
            None => return Err(SettingsError::SetupRequired),
        };
        if let Some(profile) = profile {
            settings.solved_ac_handle = profile.handle;
            settings.tier = profile.tier;
        }
        if let Some(count) = update.daily_count {
            settings.daily_count = count;
        }
        if let Some(delivery) = update.delivery {
            settings.delivery = delivery;
        }
        if let Some(time) = update.reminder_time {
            settings.reminder_time = time.into();
        }
        if let Some(goal) = update.goal {
            settings.goal = goal.trim().into();
        }
        self.write(discord_id, &settings)
            .map_err(SettingsError::Storage)?;
        Ok(settings)
    }

    fn write(&self, discord_id: u64, settings: &Settings) -> io::Result<()> {
        let path = self.path(discord_id);
        let directory = self.users.join(discord_id.to_string());
        fs::create_dir_all(&directory)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(&mut temporary, settings).map_err(io::Error::other)?;
        temporary.write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn valid_time(time: &str) -> bool {
    let [h1, h2, b':', m1, m2] = *time.as_bytes() else {
        return false;
    };
    [h1, h2, m1, m2].iter().all(u8::is_ascii_digit)
        && (h1 - b'0') * 10 + h2 - b'0' < 24
        && (m1 - b'0') * 10 + m2 - b'0' < 60
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Profiles;

    impl ProfileLookup for Profiles {
        fn lookup(&self, handle: &str) -> Result<Profile, LookupError> {
            match handle {
                "missing" => Err(LookupError::UnknownHandle),
                "offline" => Err(LookupError::Unavailable),
                _ => Ok(Profile {
                    handle: handle.into(),
                    tier: "Gold V".into(),
                }),
            }
        }
    }

    #[test]
    fn reloads_settings_and_updates_only_the_requesting_user() {
        let data = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(data.path());
        store.setup(10, " alice ", 3, &Profiles).unwrap();
        let bob = store.setup(20, "bob", 2, &Profiles).unwrap();

        let store = SettingsStore::new(data.path());
        let alice = store
            .update(
                10,
                SettingsUpdate {
                    reminder_time: Some("23:59"),
                    goal: Some("cache"),
                    ..Default::default()
                },
                &Profiles,
            )
            .unwrap();
        assert_eq!(alice.solved_ac_handle, "alice");
        assert_eq!(alice.daily_count, 3);
        assert_eq!(alice.reminder_time, "23:59");
        assert_eq!(alice.goal, "cache");
        assert_eq!(store.read(10).unwrap(), Some(alice));
        assert_eq!(store.read(20).unwrap(), Some(bob));
    }

    #[test]
    fn rejects_invalid_settings_and_preserves_data_on_lookup_failure() {
        let data = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(data.path());
        let saved = store.setup(10, "alice", 3, &Profiles).unwrap();

        for (handle, error) in [
            ("missing", LookupError::UnknownHandle),
            ("offline", LookupError::Unavailable),
        ] {
            assert!(matches!(
                store.setup(10, handle, 4, &Profiles),
                Err(SettingsError::Lookup(actual)) if actual == error
            ));
        }
        for count in [0, 11] {
            assert!(matches!(
                store.setup(10, "alice", count, &Profiles),
                Err(SettingsError::InvalidCount)
            ));
        }
        assert!(matches!(
            store.update(
                10,
                SettingsUpdate {
                    reminder_time: Some("24:00"),
                    ..Default::default()
                },
                &Profiles
            ),
            Err(SettingsError::InvalidReminderTime)
        ));
        assert_eq!(store.read(10).unwrap(), Some(saved));
    }
}

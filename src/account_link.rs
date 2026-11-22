use crate::solved_ac::{Account, AccountApi, ApiError, Authorization, Tokens};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

#[derive(Debug)]
pub enum LinkError {
    Api(ApiError),
    Storage(io::Error),
    ReconnectRequired,
    WrongAccount,
}

#[derive(Deserialize, Serialize)]
struct LinkedAccount {
    #[serde(default)]
    refresh_after: Option<u64>,
    account: Account,
    tokens: Tokens,
    expires_at: u64,
}

pub struct AccountLinks {
    data: PathBuf,
    pending: Mutex<HashMap<u64, Authorization>>,
    writes: Mutex<()>,
}

impl AccountLinks {
    pub fn new(data: &Path) -> Self {
        Self {
            data: data.join("users"),
            pending: Mutex::new(HashMap::new()),
            writes: Mutex::new(()),
        }
    }

    pub fn begin(
        &self,
        user: u64,
        redirect: &str,
        client: &impl AccountApi,
    ) -> Result<Url, LinkError> {
        let authorization = client.begin(redirect).map_err(LinkError::Storage)?;
        let url = authorization.url.clone();
        self.pending
            .lock()
            .map_err(lock_error)?
            .insert(user, authorization);
        Ok(url)
    }

    pub fn complete(
        &self,
        state: &str,
        code: &str,
        client: &impl AccountApi,
    ) -> Result<(u64, Account), LinkError> {
        let (user, authorization) = {
            let mut pending = self.pending.lock().map_err(lock_error)?;
            let user = pending
                .iter()
                .find_map(|(&user, request)| request.matches(state).then_some(user))
                .ok_or(LinkError::Api(ApiError::InvalidState))?;
            let request = pending
                .remove(&user)
                .ok_or(LinkError::Api(ApiError::InvalidState))?;
            (user, request)
        };
        let _guard = self.writes.lock().map_err(lock_error)?;
        let tokens = client
            .exchange(authorization, state, code)
            .map_err(LinkError::Api)?;
        if !tokens.permits_solves() || tokens.refresh_token.is_none() {
            return Err(LinkError::Api(ApiError::Permission));
        }
        let account = client.account(&tokens).map_err(LinkError::Api)?;
        let stored_account = Account {
            id: account.id,
            handle: account.handle.clone(),
        };
        self.write(
            user,
            &LinkedAccount {
                refresh_after: None,
                account: stored_account,
                expires_at: epoch()?.saturating_add(tokens.expires_in),
                tokens,
            },
        )?;
        Ok((user, account))
    }

    pub fn tokens_for(
        &self,
        user: u64,
        account_id: u64,
        client: &impl AccountApi,
    ) -> Result<Tokens, LinkError> {
        let _guard = self.writes.lock().map_err(lock_error)?;
        let file = File::open(self.path(user)).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                LinkError::ReconnectRequired
            } else {
                LinkError::Storage(error)
            }
        })?;
        let mut linked: LinkedAccount = serde_json::from_reader(file)
            .map_err(io::Error::other)
            .map_err(LinkError::Storage)?;
        if linked.account.id != account_id {
            return Err(LinkError::WrongAccount);
        }
        let now = epoch()?;
        if linked.expires_at > now.saturating_add(30) {
            return Ok(linked.tokens);
        }
        if let Some(after) = linked.refresh_after
            && after > now
        {
            return Err(LinkError::Api(ApiError::RateLimited {
                retry_after_seconds: Some(after - now),
            }));
        }
        let refresh = linked
            .tokens
            .refresh_token
            .take()
            .ok_or(LinkError::ReconnectRequired)?;
        // A response can be lost after the server spends the token.
        self.write(user, &linked)?;
        linked.tokens = match client.refresh(&refresh) {
            Ok(tokens) => tokens,
            Err(ApiError::RateLimited {
                retry_after_seconds,
            }) => {
                linked.tokens.refresh_token = Some(refresh);
                linked.refresh_after = Some(now.saturating_add(retry_after_seconds.unwrap_or(60)));
                self.write(user, &linked)?;
                return Err(LinkError::Api(ApiError::RateLimited {
                    retry_after_seconds,
                }));
            }
            Err(error) => return Err(LinkError::Api(error)),
        };
        linked.refresh_after = None;
        if linked.tokens.refresh_token.is_none() || !linked.tokens.permits_solves() {
            return Err(LinkError::ReconnectRequired);
        }
        linked.expires_at = now.saturating_add(linked.tokens.expires_in);
        self.write(user, &linked)?;
        Ok(linked.tokens)
    }

    pub fn account(&self, user: u64) -> Result<Account, LinkError> {
        let file = File::open(self.path(user)).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                LinkError::ReconnectRequired
            } else {
                LinkError::Storage(error)
            }
        })?;
        let linked: LinkedAccount = serde_json::from_reader(file)
            .map_err(io::Error::other)
            .map_err(LinkError::Storage)?;
        Ok(linked.account)
    }

    fn path(&self, user: u64) -> PathBuf {
        self.data
            .join(user.to_string())
            .join("solved-ac-session.json")
    }

    fn write(&self, user: u64, linked: &LinkedAccount) -> Result<(), LinkError> {
        let path = self.path(user);
        let dir = self.data.join(user.to_string());
        fs::create_dir_all(&dir).map_err(LinkError::Storage)?;
        let mut file = tempfile::NamedTempFile::new_in(dir).map_err(LinkError::Storage)?;
        serde_json::to_writer(&mut file, linked)
            .map_err(io::Error::other)
            .map_err(LinkError::Storage)?;
        file.flush().map_err(LinkError::Storage)?;
        file.as_file().sync_all().map_err(LinkError::Storage)?;
        file.persist(path)
            .map_err(|error| LinkError::Storage(error.error))?;
        Ok(())
    }
}

fn epoch() -> Result<u64, LinkError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
        .map_err(LinkError::Storage)
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> LinkError {
    LinkError::Storage(io::Error::other(error.to_string()))
}

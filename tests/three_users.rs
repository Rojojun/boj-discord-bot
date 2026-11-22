use boj_discord_bot::{
    account_link::{AccountLinks, LinkError},
    assignment::Problem,
    curriculum::{CurriculumStore, Plan, StageKey},
    pause::PauseStore,
    recommendation::{Candidate, Recommendation},
    replacement::{ReplacementSource, Replacements},
    settings::{DeliveryMode, LookupError, Profile, ProfileLookup, SettingsStore, SettingsUpdate},
    solved_ac::{Account, AccountApi, ApiError, Authorization, SolvedAc, Tokens},
    verification::{SolveSource, Verifications, VerifyError},
};
use chrono::{DateTime, Utc};
use std::{
    fs,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
fn now() -> DateTime<Utc> {
    "2026-11-22T12:00:00Z".parse().unwrap()
}
fn problem(id: &str) -> Problem {
    Problem {
        id: id.into(),
        title: id.into(),
        submission_url: format!("https://www.acmicpc.net/problem/{id}"),
    }
}
fn rec(root: &Path, user: u64, verified: bool) -> Recommendation {
    let rec = Recommendation {
        revision: 0,
        stage: Some(StageKey {
            track: "cache".into(),
            stage: "hash-map".into(),
        }),
        verified_account_id: verified.then_some(user + 100),
        handle: format!("user{user}"),
        tier: 10,
        requested: 2,
        topic: Some("해시맵".into()),
        problems: vec![problem("10"), problem("11")],
    };
    let dir = root.join(format!("users/{user}/recommendations"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("2026-11-22.json"),
        serde_json::to_vec(&rec).unwrap(),
    )
    .unwrap();
    rec
}
struct Source {
    user: u64,
    solved: Option<bool>,
    time: DateTime<Utc>,
}
impl SolveSource for Source {
    fn identity(&self, handle: &str) -> Result<u64, VerifyError> {
        if handle == format!("user{}", self.user) {
            Ok(self.user + 100)
        } else {
            Err(VerifyError::WrongAccount)
        }
    }
    fn status(&self, _: &Problem) -> Result<Option<bool>, ApiError> {
        Ok(self.solved)
    }
}
impl ReplacementSource for Source {
    fn metadata(&self, p: &Problem) -> Result<Candidate, ApiError> {
        Ok(Candidate {
            problem: p.clone(),
            level: if p.id == "9" { 9 } else { 10 },
            tags: vec!["hash_set".into()],
        })
    }
    fn now(&self) -> DateTime<Utc> {
        self.time
    }
}
struct Lookup;
impl ProfileLookup for Lookup {
    fn lookup(&self, h: &str) -> Result<Profile, LookupError> {
        Ok(Profile {
            handle: h.into(),
            tier: "10".into(),
        })
    }
}
#[test]
fn delivery_and_resume_prompts_are_isolated_and_survive_restart() {
    let root = tempfile::tempdir().unwrap();
    let settings = SettingsStore::new(root.path());
    for user in 1..=3 {
        settings
            .setup(user, &format!("user{user}"), 2, &Lookup)
            .unwrap();
    }
    settings
        .update(
            2,
            SettingsUpdate {
                delivery: Some(DeliveryMode::Dm),
                ..Default::default()
            },
            &Lookup,
        )
        .unwrap();
    let reload = SettingsStore::new(root.path());
    assert_eq!(
        reload.read(1).unwrap().unwrap().delivery,
        DeliveryMode::Channel
    );
    assert_eq!(reload.read(2).unwrap().unwrap().delivery, DeliveryMode::Dm);
    assert_eq!(
        reload.read(3).unwrap().unwrap().delivery,
        DeliveryMode::Channel
    );
    let pause = PauseStore::new(root.path());
    pause.skip(2, now(), Some(1)).unwrap();
    assert!(pause.claim_prompt(2, now()).unwrap().is_none());
    let tomorrow = now() + chrono::Duration::days(1);
    let expiry = pause.claim_prompt(2, tomorrow).unwrap().unwrap();
    assert!(
        PauseStore::new(root.path())
            .claim_prompt(2, tomorrow)
            .unwrap()
            .is_none()
    );
    pause.retry_prompt(2, expiry).unwrap();
    assert!(pause.claim_prompt(2, tomorrow).unwrap().is_some());
    pause.resume(2).unwrap();
    assert!(!pause.prompt_current(2, expiry, tomorrow).unwrap());
    assert!(pause.claim_prompt(1, tomorrow).unwrap().is_none());
    pause.skip(2, tomorrow, Some(2)).unwrap();
    pause.retry_prompt(2, expiry).unwrap();
    assert!(pause.claim_prompt(2, tomorrow).unwrap().is_none());
}
#[test]
fn verified_assignments_advance_once_and_do_not_credit_other_users_or_tracks() {
    let root = tempfile::tempdir().unwrap();
    let curriculum = CurriculumStore::new(root.path());
    let reports = Verifications::new(root.path());
    for user in 1..=3 {
        curriculum.select(user, Some("cache")).unwrap();
        rec(root.path(), user, user != 3);
    }
    let source = |user, solved| Source {
        user,
        solved,
        time: now(),
    };
    reports.check(1, now(), &source(1, Some(true))).unwrap();
    reports.check(1, now(), &source(1, Some(true))).unwrap();
    assert!(
        matches!(curriculum.plan(1).unwrap(),Plan::Learning{key,..} if key.stage=="linked-list")
    );
    reports.check(2, now(), &source(2, None)).unwrap();
    reports.check(3, now(), &source(3, Some(true))).unwrap();
    for user in [2, 3] {
        assert!(
            matches!(curriculum.plan(user).unwrap(),Plan::Learning{key,..} if key.stage=="hash-map")
        );
    }
    curriculum.select(2, Some("coding-interview")).unwrap();
    reports.check(2, now(), &source(2, Some(true))).unwrap();
    assert!(
        matches!(curriculum.plan(2).unwrap(),Plan::Learning{key,..} if key.stage=="fundamentals")
    );
    curriculum.select(2, Some("cache")).unwrap();
    assert!(
        matches!(curriculum.plan(2).unwrap(),Plan::Learning{key,..} if key.stage=="linked-list")
    );
}
#[test]
fn replacement_checks_ownership_expiry_pause_and_solves_and_invalidates_cached_report() {
    let root = tempfile::tempdir().unwrap();
    let offers = Replacements::new(root.path());
    let original = rec(root.path(), 1, true);
    rec(root.path(), 2, true);
    let candidate = Candidate {
        problem: problem("9"),
        level: 9,
        tags: vec!["hash_set".into()],
    };
    let code = offers
        .offer(1, now(), &original, 1, 101, candidate.clone())
        .unwrap();
    let source = |solved| Source {
        user: 1,
        solved,
        time: now(),
    };
    assert!(offers.apply(2, now(), &code, &source(Some(false))).is_err());
    assert!(
        offers
            .apply(
                1,
                now() + chrono::Duration::minutes(31),
                &code,
                &source(Some(false))
            )
            .is_err()
    );
    for solved in [Some(true), None] {
        assert!(offers.apply(1, now(), &code, &source(solved)).is_err());
    }
    let pause = PauseStore::new(root.path());
    pause.skip(1, now(), Some(1)).unwrap();
    assert!(offers.apply(1, now(), &code, &source(Some(false))).is_err());
    pause.resume(1).unwrap();
    let reports = Verifications::new(root.path());
    reports.check(1, now(), &source(Some(false))).unwrap();
    assert_eq!(
        offers
            .apply(1, now(), &code, &source(Some(false)))
            .unwrap()
            .id,
        "9"
    );
    assert!(reports.read(1, now()).unwrap().is_none());
    assert!(offers.apply(1, now(), &code, &source(Some(false))).is_err());
    let saved: Recommendation = serde_json::from_slice(
        &fs::read(root.path().join("users/1/recommendations/2026-11-22.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.problems[1].id, "11");
    assert_eq!(saved.stage, original.stage);
    assert_eq!(saved.revision, 1);
    let marker = root
        .path()
        .join("users/1/checkin-reminders/42/2026-11-22.json");
    fs::create_dir_all(marker.parent().unwrap()).unwrap();
    fs::write(&marker, b"keep").unwrap();
    let stale = offers
        .offer(1, now(), &original, 1, 101, candidate)
        .unwrap();
    assert!(
        offers
            .apply(1, now(), &stale, &source(Some(false)))
            .is_err()
    );
    assert_eq!(fs::read(marker).unwrap(), b"keep");
    assert_eq!(
        Verifications::new(root.path())
            .read(2, now())
            .unwrap()
            .unwrap_or_else(|| reports
                .check(
                    2,
                    now(),
                    &Source {
                        user: 2,
                        solved: Some(false),
                        time: now()
                    }
                )
                .unwrap())
            .results[0]
            .problem
            .id,
        "10"
    );
}
struct Api {
    calls: AtomicUsize,
    seen: Mutex<Vec<String>>,
    mode: u8,
}
impl AccountApi for Api {
    fn begin(&self, redirect: &str) -> std::io::Result<Authorization> {
        SolvedAc::new("test".into(), "unused".into()).begin(redirect)
    }
    fn exchange(&self, _: Authorization, _: &str, _: &str) -> Result<Tokens, ApiError> {
        unreachable!()
    }
    fn account(&self, _: &Tokens) -> Result<Account, ApiError> {
        unreachable!()
    }
    fn refresh(&self, token: &str) -> Result<Tokens, ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(token.into());
        match self.mode {
            1 => Err(ApiError::Unavailable),
            2 => Err(ApiError::RateLimited {
                retry_after_seconds: Some(120),
            }),
            _ => Ok(Tokens {
                access_token: format!("access-{token}"),
                expires_in: 3600,
                scope: "solves offline.access".into(),
                refresh_token: Some(format!("next-{token}")),
            }),
        }
    }
}
fn session(root: &Path, user: u64) {
    let dir = root.join(format!("users/{user}"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("solved-ac-session.json"),serde_json::to_vec(&serde_json::json!({"account":{"id":user+100,"handle":format!("user{user}")},"expires_at":0,"tokens":{"access_token":"expired","expires_in":0,"scope":"solves offline.access","refresh_token":format!("refresh-{user}")}})).unwrap()).unwrap();
}
#[test]
fn three_users_refresh_rotate_once_concurrently_and_reload() {
    let root = tempfile::tempdir().unwrap();
    for user in 1..=3 {
        session(root.path(), user);
    }
    let links = AccountLinks::new(root.path());
    let api = Api {
        calls: AtomicUsize::new(0),
        seen: Mutex::new(vec![]),
        mode: 0,
    };
    std::thread::scope(|scope| {
        for user in [1, 2, 3, 1, 2, 3] {
            let links = &links;
            let api = &api;
            scope.spawn(move || {
                assert_eq!(
                    links
                        .tokens_for(user, user + 100, api)
                        .unwrap()
                        .access_token,
                    format!("access-refresh-{user}")
                );
            });
        }
    });
    assert_eq!(api.calls.load(Ordering::SeqCst), 3);
    for user in 1..=3 {
        assert_eq!(
            AccountLinks::new(root.path())
                .tokens_for(user, user + 100, &api)
                .unwrap()
                .refresh_token
                .unwrap(),
            format!("next-refresh-{user}")
        );
    }
    assert!(matches!(
        links.tokens_for(1, 102, &api),
        Err(LinkError::WrongAccount)
    ));
    assert_eq!(api.calls.load(Ordering::SeqCst), 3);
}
#[test]
fn uncertain_refresh_is_not_replayed_but_explicit_rate_limit_survives_restart() {
    for mode in [1, 2] {
        let root = tempfile::tempdir().unwrap();
        session(root.path(), 1);
        let api = Api {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(vec![]),
            mode,
        };
        assert!(
            AccountLinks::new(root.path())
                .tokens_for(1, 101, &api)
                .is_err()
        );
        let result = AccountLinks::new(root.path()).tokens_for(1, 101, &api);
        if mode == 1 {
            assert!(matches!(result, Err(LinkError::ReconnectRequired)));
        } else {
            assert!(matches!(
                result,
                Err(LinkError::Api(ApiError::RateLimited { .. }))
            ));
        }
        assert_eq!(api.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn legacy_settings_and_command_options_preserve_other_users_and_pause() {
    use boj_discord_bot::discord_commands::{Commands, definitions};
    use serenity::all::CommandDataOption;
    let root = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(root.path());
    for user in 1..=3 {
        store
            .setup(user, &format!("user{user}"), 2, &Lookup)
            .unwrap();
    }
    let path = root.path().join("users/1/settings.json");
    let mut legacy: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("delivery");
    fs::write(&path, legacy.to_string()).unwrap();
    assert_eq!(
        store.read(1).unwrap().unwrap().delivery,
        DeliveryMode::Channel
    );
    PauseStore::new(root.path())
        .skip(1, now(), Some(1))
        .unwrap();
    let commands = Commands::new(root.path(), None);
    let options:Vec<CommandDataOption>=serde_json::from_value(serde_json::json!([{"name":"delivery","type":3,"value":"dm"},{"name":"count","type":4,"value":3}])).unwrap();
    assert!(
        commands
            .execute(1, "settings", &options, now())
            .contains("설정 변경 완료")
    );
    assert_eq!(store.read(1).unwrap().unwrap().daily_count, 3);
    assert_eq!(store.read(2).unwrap().unwrap().daily_count, 2);
    assert!(matches!(
        PauseStore::new(root.path()).current(1, now()).unwrap(),
        boj_discord_bot::pause::PauseStatus::SkippedUntil(_)
    ));
    let options: Vec<CommandDataOption> =
        serde_json::from_value(serde_json::json!([{"name":"goal","type":3,"value":"cache"}]))
            .unwrap();
    assert!(
        commands
            .execute(1, "settings", &options, now())
            .contains("cache")
    );
    assert!(matches!(
        CurriculumStore::new(root.path()).plan(2).unwrap(),
        Plan::General
    ));
    assert!(
        commands
            .execute(1, "version", &[], now())
            .contains(env!("CARGO_PKG_VERSION"))
    );
    let mut names = std::collections::BTreeSet::new();
    for definition in definitions() {
        let value = serde_json::to_value(definition).unwrap();
        let name = value["name"].as_str().unwrap();
        assert!(names.insert(name.to_owned()));
        assert!(value["description"].as_str().unwrap().chars().count() <= 100);
    }
    for name in ["replace", "notify-test", "version"] {
        assert!(names.contains(name));
    }
}

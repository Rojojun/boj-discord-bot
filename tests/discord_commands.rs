use boj_discord_bot::{
    discord_commands::Commands,
    settings::{LookupError, Profile, ProfileLookup, SettingsStore},
};
use serenity::all::CommandDataOption;

struct Profiles;
impl ProfileLookup for Profiles {
    fn lookup(&self, handle: &str) -> Result<Profile, LookupError> {
        Ok(Profile {
            handle: handle.into(),
            tier: "10".into(),
        })
    }
}

#[test]
fn oauth_requires_safe_exact_callback_url_and_configuration() {
    let data = tempfile::tempdir().unwrap();
    for redirect in [
        "http://example.com/oauth/callback",
        "https://user:secret@example.com/oauth/callback",
        "https://example.com/wrong",
        "https://example.com/oauth/callback?code=x",
    ] {
        assert!(
            Commands::new(data.path(), None)
                .with_redirect(redirect.into())
                .is_err()
        );
    }
    assert!(
        Commands::new(data.path(), None)
            .with_redirect("https://bot.example.com/oauth/callback".into())
            .is_ok()
    );
    let commands = Commands::new(data.path(), None);
    let now = std::time::SystemTime::now().into();
    assert!(
        commands
            .execute(1, "connect", &[], now)
            .contains("앱 연결이 필요")
    );
    assert!(!commands.complete_link("unknown", "code"));
}

fn option(name: &str, kind: u8, value: serde_json::Value) -> CommandDataOption {
    serde_json::from_value(serde_json::json!({"name":name,"type":kind,"value":value})).unwrap()
}

#[test]
fn dispatch_validates_settings_preserves_other_users_and_survives_restart() {
    let data = tempfile::tempdir().unwrap();
    let settings = SettingsStore::new(data.path());
    settings.setup(1, "alice", 3, &Profiles).unwrap();
    settings.setup(2, "bob", 2, &Profiles).unwrap();
    let commands = Commands::new(data.path(), None);
    let now = std::time::SystemTime::now().into();
    let options = [
        option("count", 4, 5.into()),
        option("reminder", 3, "22:30".into()),
    ];
    assert!(
        commands
            .execute(1, "settings", &options, now)
            .contains("설정 변경 완료")
    );
    let invalid = [option("reminder", 3, "24:00".into())];
    assert!(
        commands
            .execute(1, "settings", &invalid, now)
            .contains("00:00~23:59")
    );
    let restarted = Commands::new(data.path(), None);
    let response = restarted.execute(1, "settings", &[], now);
    assert!(response.contains("하루 문제 수: 5"));
    assert!(response.contains("22:30"));
    assert_eq!(settings.read(2).unwrap().unwrap().daily_count, 2);
    assert!(
        commands
            .execute(1, "setup", &[], now)
            .contains("앱 연결이 필요")
    );
    assert_eq!(settings.read(1).unwrap().unwrap().daily_count, 5);
}

#[test]
fn skip_resume_and_curriculum_commands_persist_across_restart() {
    let data = tempfile::tempdir().unwrap();
    let commands = Commands::new(data.path(), None);
    let now = std::time::SystemTime::now().into();
    commands.execute(1, "skip", &[option("days", 4, 2.into())], now);
    let pause = data.path().join("users/1/pause.json");
    assert!(pause.exists());
    let restarted = Commands::new(data.path(), None);
    restarted.execute(1, "resume", &[], now);
    assert!(!pause.exists());
    commands.execute(1, "curriculum", &[option("track", 3, "cache".into())], now);
    assert!(
        Commands::new(data.path(), None)
            .execute(1, "curriculum", &[], now)
            .contains("해시맵")
    );
}

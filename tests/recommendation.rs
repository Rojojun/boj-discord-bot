use boj_discord_bot::{
    assignment::Problem,
    curriculum::CurriculumStore,
    pause::PauseStore,
    recommendation::{Candidate, RecommendationSource, RecommendationStore},
    settings::{LookupError, Profile, ProfileLookup, SettingsStore},
};

struct Source {
    offline: bool,
}
impl ProfileLookup for Source {
    fn lookup(&self, handle: &str) -> Result<Profile, LookupError> {
        if self.offline {
            return Err(LookupError::Unavailable);
        }
        Ok(Profile {
            handle: handle.into(),
            tier: "10".into(),
        })
    }
}
impl RecommendationSource for Source {
    fn problems(&self, _: u8, _: &[&str], _: usize) -> Result<Vec<Candidate>, String> {
        if self.offline {
            return Err("offline".into());
        }
        Ok([
            ("a", 10, "dp"),
            ("b", 9, "hash_set"),
            ("c", 11, "dp"),
            ("a", 10, "dp"),
            ("d", 15, "dp"),
        ]
        .into_iter()
        .map(|(id, level, tag)| Candidate {
            problem: Problem {
                id: id.into(),
                title: id.into(),
                submission_url: format!("https://judge.example/{id}"),
            },
            level,
            tags: vec![tag.into()],
        })
        .collect())
    }
}

#[test]
fn preview_filters_difficulty_deduplicates_and_reuses_daily_result_without_network() {
    let data = tempfile::tempdir().unwrap();
    let source = Source { offline: false };
    let settings = SettingsStore::new(data.path());
    settings.setup(1, "alice", 4, &source).unwrap();
    let now = "2026-11-21T14:59:59Z".parse().unwrap();
    let store = RecommendationStore::new(data.path());
    let result = store.today(1, now, &settings, &source).unwrap();
    assert_eq!(
        result
            .problems
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(result.requested, 4);
    assert!(!data.path().join("users/1/assignments").exists());
    let offline = Source { offline: true };
    assert_eq!(
        RecommendationStore::new(data.path())
            .today(1, now, &settings, &offline)
            .unwrap(),
        result
    );
    let tomorrow = "2026-11-21T15:00:00Z".parse().unwrap();
    assert!(store.today(1, tomorrow, &settings, &offline).is_err());
    assert!(
        !data
            .path()
            .join("users/1/recommendations/2026-11-22.json")
            .exists()
    );
}

#[test]
fn curriculum_and_pause_apply_and_do_not_change_existing_saved_recommendations() {
    let data = tempfile::tempdir().unwrap();
    let source = Source { offline: false };
    let settings = SettingsStore::new(data.path());
    settings.setup(1, "alice", 3, &source).unwrap();
    let now = "2026-11-21T12:00:00Z".parse().unwrap();
    let curriculum = CurriculumStore::new(data.path());
    curriculum.select(1, Some("cache")).unwrap();
    let store = RecommendationStore::new(data.path());
    let saved = store.today(1, now, &settings, &source).unwrap();
    assert_eq!(
        saved
            .problems
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );
    assert_eq!(saved.topic.as_deref(), Some("해시맵"));
    curriculum.select(1, None).unwrap();
    assert_eq!(store.today(1, now, &settings, &source).unwrap(), saved);
    let pause = PauseStore::new(data.path());
    pause.skip(1, now, Some(1)).unwrap();
    assert!(
        store
            .today(1, now, &settings, &source)
            .unwrap_err()
            .contains("정지")
    );
    pause.resume(1).unwrap();
    assert_eq!(store.today(1, now, &settings, &source).unwrap(), saved);
    assert!(
        store
            .today(2, now, &settings, &source)
            .unwrap_err()
            .contains("/setup")
    );
}

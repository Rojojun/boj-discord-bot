use boj_discord_bot::curriculum::{CurriculumStore, GoalMatcher, MatchError, Plan, Track, suggest};

struct Matcher(&'static str);

impl GoalMatcher for Matcher {
    fn suggest(&self, goal: &str, tracks: &[Track]) -> Result<Option<String>, MatchError> {
        assert_eq!(goal, "LRU와 LFU를 배우고 싶다");
        assert_eq!(tracks.len(), 2);
        Ok(Some(self.0.into()))
    }
}

#[test]
fn validates_track_suggestions_and_preserves_progress_after_switching() {
    let data = tempfile::tempdir().unwrap();
    let store = CurriculumStore::new(data.path());
    let goal = "LRU와 LFU를 배우고 싶다";
    assert!(suggest(goal, &Matcher("unknown")).is_none());
    let track = suggest(goal, &Matcher("cache")).unwrap();
    assert_eq!(store.plan(10).unwrap(), Plan::General);
    store.select(10, Some(track.id)).unwrap();

    let Plan::Learning { key, stage } = store.plan(10).unwrap() else {
        panic!("missing stage")
    };
    assert_eq!(stage.id, "hash-map");
    store.record_solved(10, &key, &["a", "a"]).unwrap();
    assert!(
        matches!(store.plan(10).unwrap(), Plan::Learning { stage, .. } if stage.id == "hash-map")
    );
    store.select(10, Some("coding-interview")).unwrap();
    store.record_solved(10, &key, &["b"]).unwrap();
    let restarted = CurriculumStore::new(data.path());
    restarted.select(10, Some("cache")).unwrap();
    restarted.record_solved(10, &key, &["b"]).unwrap();
    assert!(
        matches!(restarted.plan(10).unwrap(), Plan::Learning { stage, .. } if stage.id == "linked-list")
    );
}

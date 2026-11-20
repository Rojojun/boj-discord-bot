use boj_discord_bot::assignment::{
    AssignmentError, AssignmentStore, Difficulty, Problem, ProblemLookup,
};
use boj_discord_bot::settings::{LookupError, Profile, ProfileLookup, SettingsStore};
use chrono::{DateTime, Utc};

struct Source {
    fails: bool,
}

impl ProfileLookup for Source {
    fn lookup(&self, handle: &str) -> Result<Profile, LookupError> {
        Ok(Profile {
            handle: handle.into(),
            tier: "Gold V".into(),
        })
    }
}

impl ProblemLookup for Source {
    fn candidates(
        &self,
        handle: &str,
        tier: &str,
        difficulty: Difficulty,
        _excluded: &[Problem],
        limit: usize,
    ) -> Result<Vec<Problem>, LookupError> {
        assert_eq!((handle, tier), ("alice", "Gold V"));
        let ids = match difficulty {
            Difficulty::Same => &["a"][..],
            Difficulty::OneLower if self.fails => return Err(LookupError::Unavailable),
            Difficulty::OneLower => &["a", "b"],
            Difficulty::OneHigher => &[],
        };
        Ok(ids
            .iter()
            .take(limit)
            .map(|id| Problem {
                id: (*id).into(),
                title: (*id).into(),
                submission_url: format!("https://example.test/problems/{id}"),
            })
            .collect())
    }
}

#[test]
fn persists_a_partial_assignment_and_reuses_it_until_seoul_midnight() {
    let data = tempfile::tempdir().unwrap();
    let settings = SettingsStore::new(data.path());
    let source = Source { fails: false };
    settings.setup(10, "alice", 3, &source).unwrap();
    let now: DateTime<Utc> = "2026-11-20T15:00:00Z".parse().unwrap();
    let saved = AssignmentStore::new(data.path())
        .today(10, now, &settings, &source)
        .unwrap();
    assert_eq!(
        saved
            .problems
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(saved.shortfall(), 1);
    assert!(
        data.path()
            .join("users/10/assignments/2026-11-21.json")
            .exists()
    );

    let restarted = AssignmentStore::new(data.path());
    let out_of_service = Source { fails: true };
    assert_eq!(
        restarted
            .today(10, now, &settings, &out_of_service)
            .unwrap(),
        saved
    );
    let tomorrow = now + chrono::Days::new(1);
    assert!(matches!(
        restarted.today(10, tomorrow, &settings, &out_of_service),
        Err(AssignmentError::Lookup(LookupError::Unavailable))
    ));
    assert!(
        !data
            .path()
            .join("users/10/assignments/2026-11-22.json")
            .exists()
    );
    assert_eq!(
        restarted.today(10, tomorrow, &settings, &source).unwrap(),
        saved
    );
}

#[test]
fn requires_setup_for_new_assignments() {
    let data = tempfile::tempdir().unwrap();
    let settings = SettingsStore::new(data.path());
    let now = "2026-11-20T15:00:00Z".parse().unwrap();
    assert!(matches!(
        AssignmentStore::new(data.path()).today(10, now, &settings, &Source { fails: false }),
        Err(AssignmentError::SetupRequired)
    ));
}

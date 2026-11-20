use boj_discord_bot::{
    assignment::{Assignment, AssignmentStore, Problem},
    problem_help::{
        HelpAdvice, HelpAdvisor, HelpCandidate, HelpCandidates, HelpError, HelpLookup,
        TaggedProblem, explain, replace,
    },
    reminder::SolveLookup,
    settings::LookupError,
};
use std::fs;

struct Sources(bool);

impl HelpLookup for Sources {
    fn candidates(
        &self,
        _: &str,
        original: &Problem,
        _: usize,
    ) -> Result<HelpCandidates, LookupError> {
        Ok(HelpCandidates {
            original: TaggedProblem {
                problem: original.clone(),
                level: 5,
                tags: vec!["stack".into()],
            },
            prerequisites: vec![tagged("practice", 2)],
            alternatives: vec![tagged("harder", 6), tagged("easier", 4)],
        })
    }
}

impl HelpAdvisor for Sources {
    fn advise(
        &self,
        _: &str,
        _: &[String],
        _: &[HelpCandidate<'_>],
        alternatives: &[HelpCandidate<'_>],
    ) -> Option<HelpAdvice> {
        assert_eq!(alternatives[0].id, "easier");
        Some(HelpAdvice {
            concepts: vec!["hint".into()],
            prerequisite_id: None,
            alternative_id: Some("invented".into()),
        })
    }
}

impl SolveLookup for Sources {
    fn solved(&self, _: &str, problems: &[Problem]) -> Result<Vec<Option<bool>>, LookupError> {
        assert_eq!(problems.len(), 2);
        Ok(vec![Some(self.0), Some(false)])
    }
}

fn tagged(id: &str, level: u8) -> TaggedProblem {
    TaggedProblem {
        problem: Problem {
            id: id.into(),
            title: id.into(),
            submission_url: format!("https://example.test/{id}"),
        },
        level,
        tags: vec!["stack".into()],
    }
}

#[test]
fn rejects_model_invented_ids_and_replaces_only_an_unsolved_current_slot() {
    let data = tempfile::tempdir().unwrap();
    let store = AssignmentStore::new(data.path());
    let directory = data.path().join("users/10/assignments");
    fs::create_dir_all(&directory).unwrap();
    let original = tagged("original", 5).problem;
    let day = "2026-11-20".parse().unwrap();
    fs::write(
        directory.join("2026-11-20.json"),
        serde_json::to_vec(&Assignment {
            solved_ac_handle: Some("alice".into()),
            stage: None,
            requested: 1,
            problems: vec![original],
        })
        .unwrap(),
    )
    .unwrap();
    let now = "2026-11-20T12:00:00Z".parse().unwrap();
    let sources = Sources(false);
    let offer = explain(&store, 10, now, "original", &sources, &sources).unwrap();
    assert_eq!(offer.concepts, ["stack"]);
    assert_eq!(offer.prerequisite.as_ref().unwrap().id, "practice");
    assert_eq!(offer.alternative().unwrap().id, "easier");

    assert!(matches!(
        replace(&store, &offer, 11, now, &sources),
        Err(HelpError::StaleSelection)
    ));
    assert!(matches!(
        replace(&store, &offer, 10, now, &Sources(true)),
        Err(HelpError::AlreadySolved)
    ));
    replace(&store, &offer, 10, now, &sources).unwrap();
    assert_eq!(
        store.read(10, day).unwrap().unwrap().problems[0].id,
        "easier"
    );
    assert!(matches!(
        replace(&store, &offer, 10, now, &sources),
        Err(HelpError::StaleSelection)
    ));
}

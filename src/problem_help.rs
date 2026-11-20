use crate::assignment::{AssignmentStore, Problem, seoul_time};
use crate::curriculum::{StageKey, TRACKS};
use crate::reminder::SolveLookup;
use crate::settings::LookupError;
use chrono::{DateTime, NaiveDate, Utc};
use std::io;

const MAX_CANDIDATES: usize = 10;

pub struct TaggedProblem {
    pub problem: Problem,
    pub level: u8,
    pub tags: Vec<String>,
}

pub struct HelpCandidates {
    pub original: TaggedProblem,
    pub prerequisites: Vec<TaggedProblem>,
    pub alternatives: Vec<TaggedProblem>,
}

pub trait HelpLookup {
    /// Return submittable, unsolved candidates for the given account.
    fn candidates(
        &self,
        handle: &str,
        original: &Problem,
        limit: usize,
    ) -> Result<HelpCandidates, LookupError>;
}

pub struct HelpAdvice {
    pub concepts: Vec<String>,
    pub prerequisite_id: Option<String>,
    pub alternative_id: Option<String>,
}

pub struct HelpCandidate<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub tags: &'a [String],
}

impl<'a> From<&'a TaggedProblem> for HelpCandidate<'a> {
    fn from(problem: &'a TaggedProblem) -> Self {
        Self {
            id: &problem.problem.id,
            title: &problem.problem.title,
            tags: &problem.tags,
        }
    }
}

pub trait HelpAdvisor {
    fn advise(
        &self,
        title: &str,
        tags: &[String],
        prerequisites: &[HelpCandidate<'_>],
        alternatives: &[HelpCandidate<'_>],
    ) -> Option<HelpAdvice>;
}

#[derive(Debug)]
pub enum HelpError {
    StaleSelection,
    Paused,
    AlreadySolved,
    CheckPending,
    Lookup(LookupError),
    Storage(io::Error),
}

pub struct HelpOffer {
    user: u64,
    date: NaiveDate,
    original_id: String,
    stage: Option<StageKey>,
    pub concepts: Vec<String>,
    pub prerequisite: Option<Problem>,
    alternative: Option<Problem>,
}

impl HelpOffer {
    pub fn alternative(&self) -> Option<&Problem> {
        self.alternative.as_ref()
    }
}

pub fn explain(
    assignments: &AssignmentStore,
    user: u64,
    now: DateTime<Utc>,
    original_id: &str,
    lookup: &impl HelpLookup,
    advisor: &impl HelpAdvisor,
) -> Result<HelpOffer, HelpError> {
    let date = seoul_time(now).date_naive();
    let assignment = assignments
        .read(user, date)
        .map_err(HelpError::Storage)?
        .ok_or(HelpError::StaleSelection)?;
    let original = assignment
        .problems
        .iter()
        .find(|problem| problem.id == original_id)
        .ok_or(HelpError::StaleSelection)?;
    let handle = assignment
        .solved_ac_handle
        .as_deref()
        .ok_or(HelpError::CheckPending)?;
    let candidates = lookup
        .candidates(handle, original, MAX_CANDIDATES)
        .map_err(HelpError::Lookup)?;
    if candidates.original.problem.id != original_id {
        return Err(HelpError::StaleSelection);
    }

    let available = |problem: &TaggedProblem| {
        !assignment
            .problems
            .iter()
            .any(|assigned| assigned.id == problem.problem.id)
    };
    let prerequisites: Vec<_> = candidates
        .prerequisites
        .iter()
        .filter(|problem| available(problem))
        .take(MAX_CANDIDATES)
        .collect();
    let alternatives: Vec<_> = candidates
        .alternatives
        .iter()
        .filter(|problem| {
            available(problem)
                && problem.level < candidates.original.level
                && problem
                    .tags
                    .iter()
                    .any(|tag| candidates.original.tags.contains(tag))
                && matches_stage(&problem.tags, assignment.stage.as_ref())
        })
        .take(MAX_CANDIDATES)
        .collect();

    let advice = advisor.advise(
        &original.title,
        &candidates.original.tags,
        &prerequisites
            .iter()
            .copied()
            .map(HelpCandidate::from)
            .collect::<Vec<_>>(),
        &alternatives
            .iter()
            .copied()
            .map(HelpCandidate::from)
            .collect::<Vec<_>>(),
    );
    let valid = advice.filter(|advice| {
        advice
            .prerequisite_id
            .as_ref()
            .is_none_or(|id| prerequisites.iter().any(|p| &p.problem.id == id))
            && advice
                .alternative_id
                .as_ref()
                .is_none_or(|id| alternatives.iter().any(|p| &p.problem.id == id))
            && !advice.concepts.is_empty()
            && advice.concepts.len() <= 5
            && advice
                .concepts
                .iter()
                .all(|concept| !concept.trim().is_empty() && concept.chars().count() <= 300)
    });
    let (concepts, prerequisite, alternative) = match valid {
        Some(advice) => (
            advice.concepts,
            choose(&prerequisites, advice.prerequisite_id.as_deref()),
            choose(&alternatives, advice.alternative_id.as_deref()),
        ),
        None => (
            candidates.original.tags,
            prerequisites
                .first()
                .map(|candidate| candidate.problem.clone()),
            alternatives
                .first()
                .map(|candidate| candidate.problem.clone()),
        ),
    };
    Ok(HelpOffer {
        user,
        date,
        original_id: original_id.into(),
        stage: assignment.stage,
        concepts,
        prerequisite,
        alternative,
    })
}

pub fn replace(
    assignments: &AssignmentStore,
    offer: &HelpOffer,
    user: u64,
    now: DateTime<Utc>,
    solves: &impl SolveLookup,
) -> Result<(), HelpError> {
    let date = seoul_time(now).date_naive();
    if (offer.user, offer.date) != (user, date) {
        return Err(HelpError::StaleSelection);
    }
    let alternative = offer
        .alternative
        .as_ref()
        .ok_or(HelpError::StaleSelection)?;
    assignments.replace_problem(
        user,
        date,
        &offer.original_id,
        alternative,
        |assignment, original| {
            if assignment.stage != offer.stage {
                return Err(HelpError::StaleSelection);
            }
            let handle = assignment
                .solved_ac_handle
                .as_deref()
                .ok_or(HelpError::CheckPending)?;
            match solves.solved(handle, &[original.clone(), alternative.clone()]) {
                Ok(results) if results.as_slice() == [Some(false), Some(false)] => Ok(()),
                Ok(results) if results.contains(&Some(true)) => Err(HelpError::AlreadySolved),
                _ => Err(HelpError::CheckPending),
            }
        },
    )
}

fn choose(candidates: &[&TaggedProblem], id: Option<&str>) -> Option<Problem> {
    candidates
        .iter()
        .find(|candidate| Some(candidate.problem.id.as_str()) == id)
        .map(|candidate| candidate.problem.clone())
}

fn matches_stage(tags: &[String], key: Option<&StageKey>) -> bool {
    key.is_none_or(|key| {
        TRACKS
            .iter()
            .find(|track| track.id == key.track)
            .and_then(|track| track.stages.iter().find(|stage| stage.id == key.stage))
            .is_some_and(|stage| tags.iter().any(|tag| stage.tags.contains(&tag.as_str())))
    })
}

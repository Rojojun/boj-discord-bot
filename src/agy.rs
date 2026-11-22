//! Only public learning context crosses this socket. Credentials never do.
use crate::{
    curriculum::{GoalMatcher, MatchError, Track},
    problem_help::{HelpAdvice, HelpAdvisor, HelpCandidate},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

pub struct AgyClient {
    socket: PathBuf,
}
impl AgyClient {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
    fn ask(&self, request: Value) -> Option<Value> {
        let input = serde_json::to_vec(&request).ok()?;
        if input.len() > 32768 {
            return None;
        }
        let mut stream = UnixStream::connect(&self.socket).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_secs(75)))
            .ok()?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .ok()?;
        stream.write_all(&input).ok()?;
        stream.write_all(b"\n").ok()?;
        stream.shutdown(Shutdown::Write).ok()?;
        let mut output = Vec::new();
        stream.take(65537).read_to_end(&mut output).ok()?;
        if output.len() > 65536 {
            return None;
        }
        let reply: Value = serde_json::from_slice(&output).ok()?;
        if reply.get("ok")?.as_bool()? {
            reply.get("result").cloned()
        } else {
            None
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalReply {
    track_id: Option<String>,
}
impl GoalMatcher for AgyClient {
    fn suggest(&self, goal: &str, tracks: &[Track]) -> Result<Option<String>, MatchError> {
        if goal.trim().is_empty() || goal.chars().count() > 500 {
            return Err(MatchError::Unavailable);
        }
        let request = json!({"task":"goal","goal":goal,"tracks":tracks.iter().map(|track|json!({"id":track.id,"description":track.description})).collect::<Vec<_>>()});
        let reply = self.ask(request).ok_or(MatchError::Unavailable)?;
        parse_goal(reply, tracks)
    }
}
fn parse_goal(reply: Value, tracks: &[Track]) -> Result<Option<String>, MatchError> {
    let reply: GoalReply = serde_json::from_value(reply).map_err(|_| MatchError::Unavailable)?;
    if reply
        .track_id
        .as_ref()
        .is_some_and(|id| !tracks.iter().any(|track| track.id == id))
    {
        return Err(MatchError::Unavailable);
    }
    Ok(reply.track_id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdviceReply {
    concepts: Vec<String>,
    prerequisite_id: Option<String>,
    alternative_id: Option<String>,
}
fn parse_advice(
    reply: Value,
    prerequisites: &[HelpCandidate<'_>],
    alternatives: &[HelpCandidate<'_>],
) -> Option<HelpAdvice> {
    let reply: AdviceReply = serde_json::from_value(reply).ok()?;
    if reply.concepts.is_empty()
        || reply.concepts.len() > 5
        || reply.concepts.iter().any(|concept| {
            concept.trim().is_empty()
                || concept.chars().count() > 300
                || concept.contains("://")
                || concept.to_ascii_lowercase().contains("www.")
                || concept.contains("<@")
        })
        || reply
            .prerequisite_id
            .as_ref()
            .is_some_and(|id| !prerequisites.iter().any(|p| p.id == id))
        || reply
            .alternative_id
            .as_ref()
            .is_some_and(|id| !alternatives.iter().any(|p| p.id == id))
    {
        return None;
    }
    Some(HelpAdvice {
        concepts: reply.concepts,
        prerequisite_id: reply.prerequisite_id,
        alternative_id: reply.alternative_id,
    })
}
impl HelpAdvisor for AgyClient {
    fn advise(
        &self,
        title: &str,
        tags: &[String],
        prerequisites: &[HelpCandidate<'_>],
        alternatives: &[HelpCandidate<'_>],
    ) -> Option<HelpAdvice> {
        let encode = |items: &[HelpCandidate<'_>]| {
            items
                .iter()
                .map(|p| json!({"id":p.id,"title":p.title,"tags":p.tags}))
                .collect::<Vec<_>>()
        };
        parse_advice(self.ask(json!({"task":"help","title":title,"tags":tags,"prerequisites":encode(prerequisites),"alternatives":encode(alternatives)}))?, prerequisites,alternatives)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_cannot_invent_track_or_candidate_ids() {
        assert_eq!(
            parse_goal(json!({"track_id":"cache"}), crate::curriculum::TRACKS).unwrap(),
            Some("cache".into())
        );
        assert!(parse_goal(json!({"track_id":"unknown"}), crate::curriculum::TRACKS).is_err());
        assert!(
            parse_goal(
                json!({"track_id":"cache","write_file":"x"}),
                crate::curriculum::TRACKS
            )
            .is_err()
        );
        let tags = vec!["dp".into()];
        let candidates = [HelpCandidate {
            id: "10",
            title: "p",
            tags: &tags,
        }];
        let valid = json!({"concepts":["작은 상태부터 점화식을 세워보세요."],"prerequisite_id":"10","alternative_id":null});
        assert!(parse_advice(valid.clone(), &candidates, &[]).is_some());
        let mut invalid = valid.clone();
        invalid["alternative_id"] = json!("99");
        assert!(parse_advice(invalid, &candidates, &[]).is_none());
        let mut invalid = valid;
        invalid["concepts"] = json!(["https://untrusted.example"]);
        assert!(parse_advice(invalid, &candidates, &[]).is_none());
    }
}

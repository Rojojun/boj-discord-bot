use crate::{
    assignment::{Problem, seoul_time},
    curriculum::TRACKS,
    pause::{self, PauseStatus},
    recommendation::{Candidate, RECOMMENDATION_WRITES, Recommendation},
    solved_ac::ApiError,
    verification::{OAuthSolves, SolveSource},
};
use chrono::{DateTime, NaiveDate, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Offer {
    user: u64,
    date: NaiveDate,
    expires: DateTime<Utc>,
    revision: u64,
    slot: usize,
    original_id: String,
    account_id: u64,
    candidate: Candidate,
}
pub trait ReplacementSource: SolveSource {
    fn metadata(&self, problem: &Problem) -> Result<Candidate, ApiError>;
    fn now(&self) -> DateTime<Utc> {
        std::time::SystemTime::now().into()
    }
}
impl ReplacementSource for OAuthSolves<'_> {
    fn metadata(&self, problem: &Problem) -> Result<Candidate, ApiError> {
        self.api.tagged_problem(problem)
    }
}
pub struct Replacements {
    data: PathBuf,
}
impl Replacements {
    pub fn new(data: &Path) -> Self {
        Self { data: data.into() }
    }
    fn directory(&self, user: u64) -> PathBuf {
        self.data
            .join("users")
            .join(user.to_string())
            .join("replacement-offers")
    }
    pub fn offer(
        &self,
        user: u64,
        now: DateTime<Utc>,
        recommendation: &Recommendation,
        slot: usize,
        account_id: u64,
        candidate: Candidate,
    ) -> io::Result<String> {
        let original = recommendation
            .problems
            .get(
                slot.checked_sub(1)
                    .ok_or_else(|| io::Error::other("invalid slot"))?,
            )
            .ok_or_else(|| io::Error::other("invalid slot"))?;
        let mut bytes = [0u8; 16];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| io::Error::other("random source unavailable"))?;
        let id: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let offer = Offer {
            user,
            date: seoul_time(now).date_naive(),
            expires: now + chrono::Duration::minutes(30),
            revision: recommendation.revision,
            slot,
            original_id: original.id.clone(),
            account_id,
            candidate,
        };
        let directory = self.directory(user);
        fs::create_dir_all(&directory)?;
        atomic(&directory.join(format!("{id}.json")), &offer)?;
        Ok(id)
    }
    pub fn apply(
        &self,
        user: u64,
        now: DateTime<Utc>,
        id: &str,
        source: &impl ReplacementSource,
    ) -> Result<Problem, String> {
        if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("유효한 교체 코드가 아닙니다. /hint에서 새 후보를 받아주세요.".into());
        }
        let _guard = RECOMMENDATION_WRITES
            .lock()
            .map_err(|_| "추천 저장 잠금 실패")?;
        let offer: Offer = serde_json::from_reader(
            File::open(self.directory(user).join(format!("{id}.json")))
                .map_err(|_| "내 교체 제안을 찾지 못했습니다. /hint로 다시 받아주세요.")?,
        )
        .map_err(|_| "교체 제안 읽기 실패")?;
        let date = seoul_time(now).date_naive();
        if offer.user != user || offer.date != date || now > offer.expires {
            return Err("날짜가 바뀌었거나 제안이 만료됐습니다. /hint로 다시 받아주세요.".into());
        }
        let active = || -> Result<(), String> {
            if pause::status(&self.data.join("users"), user, date)
                .map_err(|_| "스킵 상태 읽기 실패")?
                != PauseStatus::Active
            {
                Err("학습 중단 중에는 교체하지 않습니다. 재개 후 새 제안을 선택하세요.".into())
            } else {
                Ok(())
            }
        };
        active()?;
        let path = self
            .data
            .join("users")
            .join(user.to_string())
            .join("recommendations")
            .join(format!("{date}.json"));
        let mut recommendation: Recommendation =
            serde_json::from_reader(File::open(&path).map_err(|_| "오늘 추천을 찾지 못했습니다.")?)
                .map_err(|_| "추천 읽기 실패")?;
        let slot = offer
            .slot
            .checked_sub(1)
            .ok_or("추천 슬롯이 바뀌었습니다.")?;
        let original = recommendation
            .problems
            .get(slot)
            .ok_or("추천 슬롯이 바뀌었습니다.")?
            .clone();
        if recommendation.revision != offer.revision
            || original.id != offer.original_id
            || recommendation
                .problems
                .iter()
                .any(|p| p.id == offer.candidate.problem.id)
        {
            return Err("다른 교체로 만료된 제안입니다. /hint로 다시 받아주세요.".into());
        }
        let owner = source
            .identity(&recommendation.handle)
            .map_err(|_| "연결 계정 확인 실패. /account를 확인하세요.")?;
        if owner != offer.account_id
            || recommendation
                .verified_account_id
                .is_some_and(|id| id != owner)
        {
            return Err("제안을 받은 계정과 현재 연결 계정이 다릅니다.".into());
        }
        let old = source
            .metadata(&original)
            .map_err(|_| "원문제 정보 확인 실패")?;
        let candidate = source
            .metadata(&offer.candidate.problem)
            .map_err(|_| "후보 문제 정보 확인 실패")?;
        let stage_ok = if let Some(key) = &recommendation.stage {
            TRACKS
                .iter()
                .find(|track| track.id == key.track)
                .and_then(|track| track.stages.iter().find(|stage| stage.id == key.stage))
                .is_some_and(|stage| {
                    candidate
                        .tags
                        .iter()
                        .any(|tag| stage.tags.contains(&tag.as_str()))
                })
        } else {
            recommendation.topic.as_ref().is_none_or(|topic| {
                TRACKS.iter().flat_map(|track| track.stages).any(|stage| {
                    stage.concept == topic
                        && candidate
                            .tags
                            .iter()
                            .any(|tag| stage.tags.contains(&tag.as_str()))
                })
            })
        };
        if candidate.problem.id != offer.candidate.problem.id
            || old.problem.id != original.id
            || candidate.level >= old.level
            || !candidate.tags.iter().any(|tag| old.tags.contains(tag))
            || !stage_ok
        {
            return Err(
                "난이도·주제 조건이 바뀌어 교체를 보류했습니다. /hint로 다시 확인하세요.".into(),
            );
        }
        for problem in [&original, &candidate.problem] {
            match source.status(problem) {
                Ok(Some(false)) => {}
                Ok(Some(true)) => {
                    return Err("원문제 또는 후보가 이미 해결됐습니다. 교체하지 않았습니다.".into());
                }
                _ => return Err("풀이 여부를 확인하지 못해 교체를 보류했습니다.".into()),
            }
        }
        let finished = source.now();
        if seoul_time(finished).date_naive() != date || finished > offer.expires {
            return Err("확인 도중 제안이 만료됐습니다. /hint로 다시 받아주세요.".into());
        }
        active()?;
        recommendation.problems[slot] = candidate.problem.clone();
        recommendation.revision = recommendation
            .revision
            .checked_add(1)
            .ok_or("추천 버전 오류")?;
        atomic(&path, &recommendation).map_err(|_| "추천 교체 저장 실패")?;
        Ok(candidate.problem)
    }
}
fn atomic(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut file, value).map_err(io::Error::other)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

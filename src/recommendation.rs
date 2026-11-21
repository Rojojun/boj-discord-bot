//! OAuth-free recommendations are previews, never confirmed-unsolved assignments.
use crate::{
    assignment::{Problem, seoul_time},
    curriculum::{CurriculumStore, Plan},
    pause::{self, PauseStatus},
    settings::{ProfileLookup, SettingsStore},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug)]
pub struct Candidate {
    pub problem: Problem,
    pub level: u8,
    pub tags: Vec<String>,
}

pub trait RecommendationSource: ProfileLookup {
    fn problems(&self) -> Result<Vec<Candidate>, String>;
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Recommendation {
    pub handle: String,
    pub tier: u8,
    pub requested: u8,
    pub topic: Option<String>,
    pub problems: Vec<Problem>,
}

pub struct RecommendationStore {
    data: PathBuf,
    writes: Mutex<()>,
}

impl RecommendationStore {
    pub fn new(data: &Path) -> Self {
        Self {
            data: data.into(),
            writes: Mutex::new(()),
        }
    }

    pub fn today(
        &self,
        user: u64,
        now: DateTime<Utc>,
        settings: &SettingsStore,
        source: &impl RecommendationSource,
    ) -> Result<Recommendation, String> {
        let date = seoul_time(now).date_naive();
        let users = self.data.join("users");
        let active = || match pause::status(&users, user, date) {
            Ok(PauseStatus::Active) => Ok(()),
            Ok(_) => Err("학습 정지 중입니다. /resume으로 재개해주세요.".to_owned()),
            Err(_) => Err("학습 정지 상태를 읽지 못했습니다.".to_owned()),
        };
        active()?;
        let directory = users.join(user.to_string()).join("recommendations");
        let path = directory.join(format!("{date}.json"));
        if let Some(saved) = read(&path).map_err(storage)? {
            return Ok(saved);
        }
        let settings = settings
            .read(user)
            .map_err(storage)?
            .ok_or("먼저 /setup으로 핸들과 문제 수를 설정하세요.")?;
        let profile = source
            .lookup(&settings.solved_ac_handle)
            .map_err(|_| "현재 티어 조회에 실패했습니다. 잠시 후 다시 시도해주세요.")?;
        let tier: u8 = profile.tier.parse().ok().filter(|tier| (1..=30).contains(tier)).ok_or("현재 티어에 맞춘 추천이 불가능합니다. Unrated·Master 계정의 난이도 선택은 아직 지원하지 않습니다.")?;
        let (topic, tags) = match CurriculumStore::new(&self.data)
            .plan(user)
            .map_err(storage)?
        {
            Plan::General => (None, &[][..]),
            Plan::Learning { stage, .. } => (Some(stage.concept.to_owned()), stage.tags),
            Plan::Completed { .. } => return Err(
                "커리큘럼을 완료했습니다. /curriculum으로 다른 트랙이나 일반 모드를 선택하세요."
                    .into(),
            ),
        };
        let candidates = source.problems()?;
        let mut problems = Vec::new();
        for level in [
            Some(tier),
            tier.checked_sub(1).filter(|v| *v > 0),
            tier.checked_add(1).filter(|v| *v <= 30),
        ]
        .into_iter()
        .flatten()
        {
            for candidate in &candidates {
                if candidate.level == level
                    && (tags.is_empty()
                        || candidate
                            .tags
                            .iter()
                            .any(|tag| tags.contains(&tag.as_str())))
                    && !problems
                        .iter()
                        .any(|p: &Problem| p.id == candidate.problem.id)
                    && problems.len() < usize::from(settings.daily_count)
                {
                    problems.push(candidate.problem.clone());
                }
            }
        }
        if problems.is_empty() {
            return Err("조회한 후보에서 현재 난이도·주제에 맞고 제출 링크가 있는 문제를 찾지 못했습니다. 다음 조회에서 다시 시도할 수 있습니다.".into());
        }
        let recommendation = Recommendation {
            handle: profile.handle,
            tier,
            requested: settings.daily_count,
            topic,
            problems,
        };
        let _guard = self.writes.lock().map_err(|_| "추천 저장 잠금 오류")?;
        active()?;
        if let Some(saved) = read(&path).map_err(storage)? {
            return Ok(saved);
        }
        fs::create_dir_all(&directory).map_err(storage)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(storage)?;
        serde_json::to_writer(&mut temporary, &recommendation).map_err(|_| "추천 직렬화 실패")?;
        temporary.flush().map_err(storage)?;
        temporary.as_file().sync_all().map_err(storage)?;
        temporary.persist(path).map_err(|_| "추천 저장 실패")?;
        Ok(recommendation)
    }
}

fn read(path: &Path) -> io::Result<Option<Recommendation>> {
    match File::open(path) {
        Ok(file) => serde_json::from_reader(file)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn storage(_: io::Error) -> String {
    "추천 데이터를 읽거나 저장하지 못했습니다. 서버 폴더 권한을 확인하세요.".into()
}

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static WRITES: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stage {
    pub id: &'static str,
    pub concept: &'static str,
    pub tags: &'static [&'static str],
    pub required: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Track {
    pub id: &'static str,
    pub description: &'static str,
    pub stages: &'static [Stage],
}

pub const TRACKS: &[Track] = &[
    Track {
        id: "cache",
        description: "해시맵부터 LRU와 LFU 캐시 구현까지",
        stages: &[
            Stage {
                id: "hash-map",
                concept: "해시맵",
                tags: &["hash_set"],
                required: 2,
            },
            Stage {
                id: "linked-list",
                concept: "이중 연결 리스트",
                tags: &["linked_list"],
                required: 2,
            },
            Stage {
                id: "lru",
                concept: "최근 사용 순서와 O(1) 갱신",
                tags: &["implementation", "linked_list"],
                required: 2,
            },
            Stage {
                id: "frequency",
                concept: "빈도별 묶음과 동률 처리",
                tags: &["hash_set", "data_structures"],
                required: 2,
            },
            Stage {
                id: "lfu",
                concept: "최소 사용 빈도와 동률 시 최근 사용 순서",
                tags: &["implementation", "data_structures"],
                required: 2,
            },
        ],
    },
    Track {
        id: "coding-interview",
        description: "배열과 해시부터 그래프와 DP까지 코딩 기술면접 준비",
        stages: &[
            Stage {
                id: "fundamentals",
                concept: "배열·문자열·해시",
                tags: &["string", "hash_set"],
                required: 2,
            },
            Stage {
                id: "windows",
                concept: "투 포인터·슬라이딩 윈도우",
                tags: &["two_pointer", "sliding_window"],
                required: 2,
            },
            Stage {
                id: "stack-queue",
                concept: "스택·큐",
                tags: &["stack", "queue"],
                required: 2,
            },
            Stage {
                id: "graphs",
                concept: "트리·그래프",
                tags: &["trees", "graphs"],
                required: 2,
            },
            Stage {
                id: "search",
                concept: "힙·이분 탐색",
                tags: &["priority_queue", "binary_search"],
                required: 2,
            },
            Stage {
                id: "dp",
                concept: "동적 계획법",
                tags: &["dp"],
                required: 2,
            },
        ],
    },
];

#[derive(Debug, PartialEq, Eq)]
pub enum MatchError {
    Unavailable,
}

pub trait GoalMatcher {
    fn suggest(&self, goal: &str, tracks: &[Track]) -> Result<Option<String>, MatchError>;
}

pub fn suggest(goal: &str, matcher: &impl GoalMatcher) -> Option<&'static Track> {
    if goal.trim().is_empty() {
        return None;
    }
    let id = matcher.suggest(goal.trim(), TRACKS).ok()??;
    TRACKS.iter().find(|track| track.id == id)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageKey {
    pub track: String,
    pub stage: String,
}

#[derive(Default, Deserialize, Serialize)]
struct Progress {
    active: Option<String>,
    solved: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
}

impl Progress {
    fn current(&self, track: &Track) -> Option<Stage> {
        let solved = self.solved.get(track.id);
        track.stages.iter().copied().find(|stage| {
            solved
                .and_then(|steps| steps.get(stage.id))
                .map_or(0, BTreeSet::len)
                < stage.required
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    General,
    Learning { key: StageKey, stage: Stage },
    Completed { track: String },
}

pub struct CurriculumStore {
    users: PathBuf,
}

impl CurriculumStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            users: data_dir.join("users"),
        }
    }

    pub fn plan(&self, user: u64) -> io::Result<Plan> {
        let progress = self.read(user)?;
        let Some(track) = TRACKS
            .iter()
            .find(|track| Some(track.id) == progress.active.as_deref())
        else {
            return Ok(Plan::General);
        };
        Ok(match progress.current(track) {
            Some(stage) => Plan::Learning {
                key: StageKey {
                    track: track.id.into(),
                    stage: stage.id.into(),
                },
                stage,
            },
            None => Plan::Completed {
                track: track.id.into(),
            },
        })
    }

    pub fn select(&self, user: u64, track: Option<&str>) -> io::Result<()> {
        if track.is_some_and(|id| !TRACKS.iter().any(|track| track.id == id)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown curriculum track",
            ));
        }
        let _guard = WRITES
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut progress = self.read(user)?;
        progress.active = track.map(str::to_owned);
        self.write(user, &progress)
    }

    /// Call with the stage recorded on the assignment and its confirmed solved IDs.
    pub fn record_solved(&self, user: u64, key: &StageKey, ids: &[&str]) -> io::Result<()> {
        let Some(track) = TRACKS.iter().find(|track| track.id == key.track) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown curriculum track",
            ));
        };
        let _guard = WRITES
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let mut progress = self.read(user)?;
        if progress
            .current(track)
            .is_none_or(|stage| stage.id != key.stage)
            || ids.is_empty()
        {
            return Ok(());
        }
        let solved = progress
            .solved
            .entry(key.track.clone())
            .or_default()
            .entry(key.stage.clone())
            .or_default();
        let previous = solved.len();
        solved.extend(ids.iter().map(|id| (*id).to_owned()));
        if solved.len() != previous {
            self.write(user, &progress)?;
        }
        Ok(())
    }

    fn path(&self, user: u64) -> PathBuf {
        self.users.join(user.to_string()).join("curriculum.json")
    }

    fn read(&self, user: u64) -> io::Result<Progress> {
        match File::open(self.path(user)) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Progress::default()),
            Err(error) => Err(error),
        }
    }

    fn write(&self, user: u64, progress: &Progress) -> io::Result<()> {
        let path = self.path(user);
        let directory = self.users.join(user.to_string());
        fs::create_dir_all(&directory)?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer_pretty(&mut file, progress).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

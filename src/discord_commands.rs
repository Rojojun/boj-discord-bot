use crate::account_link::{AccountLinks, LinkError};
use crate::agy::AgyClient;
use crate::checkin::{CheckinError, Checkins, Notice, Progress, ReminderProgress};
use crate::curriculum::{self, CurriculumStore, Plan, TRACKS};
use crate::pause::{PauseError, PauseStatus, PauseStore};
use crate::problem_help::{HelpAdvisor, HelpCandidate};
use crate::recommendation::{Candidate, Recommendation, RecommendationSource, RecommendationStore};
use crate::settings::{
    LookupError, Profile, ProfileLookup, Settings, SettingsError, SettingsStore, SettingsUpdate,
};
use crate::solved_ac::{LinkedRecommendations, SolvedAc};
use crate::verification::{
    OAuthSolves, PendingReason, Report, Verifications, VerifiedStatus, VerifyError,
};
use chrono::{DateTime, Utc};
use serenity::all::{CommandDataOption, CommandOptionType, CreateCommand, CreateCommandOption};
use std::path::Path;

pub struct Commands {
    settings: SettingsStore,
    pause: PauseStore,
    curriculum: CurriculumStore,
    api: Option<SolvedAc>,
    links: AccountLinks,
    redirect: Option<String>,
    recommendations: RecommendationStore,
    checkins: Checkins,
    verifications: Verifications,
    agy: Option<AgyClient>,
}

const STORAGE_ERROR: &str =
    "설정을 읽거나 저장하지 못했습니다. 관리자에게 서버 데이터 폴더 권한 확인을 요청하세요.";
const API_REQUIRED: &str = "solved.ac 앱 연결이 필요합니다. 관리자가 서버 환경변수 SOLVED_AC_CLIENT_ID와 SOLVED_AC_CLIENT_SECRET을 설정한 뒤 봇을 재시작해주세요.";

impl Commands {
    pub fn new(data: &Path, api: Option<SolvedAc>) -> Self {
        Self {
            settings: SettingsStore::new(data),
            pause: PauseStore::new(data),
            curriculum: CurriculumStore::new(data),
            api,
            links: AccountLinks::new(data),
            redirect: None,
            recommendations: RecommendationStore::new(data),
            checkins: Checkins::new(data),
            verifications: Verifications::new(data),
            agy: None,
        }
    }

    pub fn with_agy(mut self, client: AgyClient) -> Self {
        self.agy = Some(client);
        self
    }

    fn hint(&self, user: u64, now: DateTime<Utc>, index: usize) -> String {
        let saved = match self.checkins.progress(user,now) {
            Ok(Some(progress)) => progress.recommendation,
            Ok(None) => return "오늘 저장된 추천이 없습니다. 다음 학습일에 /today로 추천을 받은 뒤 /hint를 사용하세요. 학습 중단 상태를 해제할 필요는 없습니다.".into(),
            Err(_) => return STORAGE_ERROR.into(),
        };
        let Some(original) = index
            .checked_sub(1)
            .and_then(|index| saved.problems.get(index))
        else {
            return "/today에 표시된 문제 번호를 선택해주세요.".into();
        };
        let Some(api) = &self.api else {
            return API_REQUIRED.into();
        };
        let original = match api.tagged_problem(original) {
            Ok(original) => original,
            Err(_) => {
                return "문제 제목·태그 조회에 실패했습니다. 잠시 후 다시 시도해주세요.".into();
            }
        };
        let mut candidates = Vec::new();
        if original.level > 1
            && !original.tags.is_empty()
            && let Ok(account) = self.links.account(user)
            && saved.verified_account_id.is_none_or(|id| id == account.id)
            && let Ok(tokens) = self.links.tokens_for(user, account.id, api)
        {
            let source = LinkedRecommendations {
                api,
                tokens,
                account_id: account.id,
            };
            if source.validate_handle(&saved.handle).is_ok() {
                let tags: Vec<_> = original.tags.iter().map(String::as_str).collect();
                if let Ok(found) = source.problems(original.level - 1, &tags, 10) {
                    candidates = found
                        .into_iter()
                        .filter(|p| {
                            p.level < original.level
                                && !saved
                                    .problems
                                    .iter()
                                    .any(|assigned| assigned.id == p.problem.id)
                                && p.tags.iter().any(|tag| original.tags.contains(tag))
                                && saved.topic.as_ref().is_none_or(|topic| {
                                    TRACKS.iter().flat_map(|track| track.stages).any(|stage| {
                                        stage.concept == topic
                                            && p.tags
                                                .iter()
                                                .any(|tag| stage.tags.contains(&tag.as_str()))
                                    })
                                })
                        })
                        .take(10)
                        .collect();
                }
            }
        }
        let choices: Vec<_> = candidates
            .iter()
            .map(|p: &Candidate| HelpCandidate {
                id: &p.problem.id,
                title: &p.problem.title,
                tags: &p.tags,
            })
            .collect();
        let advice = self.agy.as_ref().and_then(|agy| {
            agy.advise(&original.problem.title, &original.tags, &choices, &choices)
        });
        let (concepts, prerequisite, alternative, label) = match advice {
            Some(advice) => (
                advice.concepts,
                advice.prerequisite_id,
                advice.alternative_id,
                "agy 제목·태그 기반 학습 힌트",
            ),
            None => (
                original
                    .tags
                    .iter()
                    .map(|tag| format!("{tag}: 이 개념의 기본 연산과 적용 조건부터 복습해보세요."))
                    .collect(),
                candidates.first().map(|p| p.problem.id.clone()),
                candidates.first().map(|p| p.problem.id.clone()),
                "AI를 사용할 수 없어 태그·검증된 후보로 안내합니다",
            ),
        };
        let mut lines = vec![
            format!("**{} · {}번 {}**", label, index, original.problem.title),
            "**선수 개념·접근 힌트**".into(),
        ];
        if concepts.is_empty() {
            lines.push("표시할 선수 개념 정보가 부족합니다.".into());
        }
        for concept in concepts {
            lines.push(format!("• {concept}"));
        }
        for (label, id) in [
            ("선수 연습", prerequisite),
            ("더 쉬운 대체 후보", alternative),
        ] {
            if let Some(candidate) =
                id.and_then(|id| candidates.iter().find(|p| p.problem.id == id))
            {
                lines.push(format!(
                    "**{}**: {}\n<{}>",
                    label, candidate.problem.title, candidate.problem.submission_url
                ));
            } else {
                lines.push(format!("**{label}**: 검증된 미해결 후보가 부족합니다."));
            }
        }
        lines.push("제목·태그 기반 학습 제안입니다. 연습 링크를 열어도 오늘 추천·체크·커리큘럼은 변경되지 않습니다. 현재 대체 후보는 안내만 하며 슬롯 교체는 지원하지 않습니다.".into());
        lines.join("\n")
    }

    pub fn with_redirect(mut self, redirect: String) -> Result<Self, std::io::Error> {
        let url = url::Url::parse(&redirect).map_err(std::io::Error::other)?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(url.scheme() == "https" || (url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/oauth/callback"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(std::io::Error::other(
                "SOLVED_AC_REDIRECT_URI는 HTTPS의 /oauth/callback 주소 또는 로컬 HTTP 주소여야 합니다.",
            ));
        }
        self.redirect = Some(redirect);
        Ok(self)
    }

    pub fn complete_link(&self, state: &str, code: &str) -> bool {
        self.api
            .as_ref()
            .is_some_and(|api| self.links.complete(state, code, api).is_ok())
    }

    pub fn checkins(&self) -> &Checkins {
        &self.checkins
    }

    fn reminder_progress(
        &self,
        user: u64,
        now: DateTime<Utc>,
        progress: Progress,
        fresh: bool,
    ) -> std::io::Result<Option<ReminderProgress>> {
        let account = match self.links.account(user) {
            Ok(account) => account,
            Err(LinkError::ReconnectRequired) => return Ok(ReminderProgress::manual(&progress)),
            Err(_) => return Err(std::io::Error::other("linked account storage unavailable")),
        };
        let cached = self.verifications.read(user, now)?.filter(|report| {
            report.account_id == account.id
                && report.handle == progress.recommendation.handle
                && progress
                    .recommendation
                    .verified_account_id
                    .is_none_or(|id| id == account.id)
                && report.results.len() == progress.total()
                && report
                    .results
                    .iter()
                    .zip(&progress.recommendation.problems)
                    .all(|(row, problem)| row.problem.id == problem.id)
        });
        if let Some(report) = &cached {
            // A solved confirmation remains valid for today's immutable problem set.
            if report
                .results
                .iter()
                .all(|row| row.status == VerifiedStatus::Solved)
            {
                return Ok(None);
            }
            let age = now.signed_duration_since(report.checked_at).num_seconds();
            if (0..300).contains(&age)
                && report
                    .results
                    .iter()
                    .any(|row| row.status == VerifiedStatus::Pending)
            {
                return Ok(None);
            }
            if !fresh && (0..120).contains(&age) {
                return Ok(ReminderProgress::from_report(report));
            }
        }
        let api = self
            .api
            .as_ref()
            .ok_or_else(|| std::io::Error::other("app credentials unavailable"))?;
        let tokens = self
            .links
            .tokens_for(user, account.id, api)
            .map_err(|_| std::io::Error::other("OAuth refresh unavailable"))?;
        let report = self
            .verifications
            .check(
                user,
                now,
                &OAuthSolves {
                    api,
                    tokens,
                    account_id: account.id,
                },
            )
            .map_err(|_| std::io::Error::other("solve verification unavailable"))?;
        Ok(ReminderProgress::from_report(&report))
    }

    pub fn claim_reminders(
        &self,
        channel: u64,
        now: DateTime<Utc>,
    ) -> std::io::Result<Vec<Notice>> {
        self.checkins
            .claim_due_with(channel, now, |user, progress| {
                self.reminder_progress(user, now, progress, true)
            })
    }
    pub fn refresh_reminder(
        &self,
        notice: &Notice,
        now: DateTime<Utc>,
    ) -> std::io::Result<Option<ReminderProgress>> {
        self.checkins.still_due_with(notice, now, |user, progress| {
            self.reminder_progress(user, now, progress, false)
        })
    }

    fn today_recommendation(
        &self,
        user: u64,
        now: DateTime<Utc>,
        api: &SolvedAc,
    ) -> Result<Recommendation, String> {
        // Preserve the saved set and skip state before any OAuth/API work.
        if let Some(saved) = self.recommendations.existing(user, now)? {
            return Ok(saved);
        }
        match self.links.account(user) {
            Ok(account) => {
                let tokens = self
                    .links
                    .tokens_for(user, account.id, api)
                    .map_err(|_| "계정 인증을 갱신하지 못했습니다. /connect로 다시 연결해주세요. 풀이 미확인 추천으로 자동 전환하지는 않습니다.".to_owned())?;
                self.recommendations.today(
                    user,
                    now,
                    &self.settings,
                    &LinkedRecommendations {
                        api,
                        tokens,
                        account_id: account.id,
                    },
                )
            }
            Err(LinkError::ReconnectRequired) => {
                self.recommendations.today(user, now, &self.settings, api)
            }
            Err(_) => Err(STORAGE_ERROR.into()),
        }
    }

    pub fn execute(
        &self,
        user: u64,
        name: &str,
        options: &[CommandDataOption],
        now: DateTime<Utc>,
    ) -> String {
        match name {
            "goal" => {
                let Some(goal)=string(options,"text") else { return "배우고 싶은 목표를 입력하세요. 예: LRU와 LFU를 배우고 싶어요".into() };
                let suggested=self.agy.as_ref().and_then(|agy|curriculum::suggest(goal,agy));
                match suggested {
                    Some(track)=>format!("**추천 트랙: {}**\n{}\n적용하려면 `/curriculum track:{}`를 직접 선택하세요. 아직 목표나 진행 상태를 변경하지 않았습니다.",track.id,track.description,track.id),
                    None=>"AI 제안을 만들지 못했거나 지원하는 트랙과 일치하지 않습니다. 직접 선택하세요:\n`/curriculum track:cache` — LRU·LFU 캐시 구현\n`/curriculum track:coding-interview` — 코딩 기술면접\n`/curriculum track:tier` — 일반 티어 추천\n기존 설정과 학습 중단 상태는 유지됩니다.".into(),
                }
            }
            "hint" => self.hint(user,now,integer(options,"problem").and_then(|value|usize::try_from(value).ok()).unwrap_or(1)),
            "check" => {
                let Some(api) = &self.api else { return API_REQUIRED.into() };
                match self.checkins.progress(user, now) {
                    Ok(None) => return "먼저 /today로 오늘 추천 문제를 받아주세요.".into(),
                    Err(_) => return STORAGE_ERROR.into(),
                    _ => {},
                }
                let account = match self.links.account(user) {
                    Ok(account) => account,
                    Err(_) => return "/connect로 본인 solved.ac 계정을 먼저 연결해주세요.".into(),
                };
                let tokens = match self.links.tokens_for(user, account.id, api) {
                    Ok(tokens) => tokens,
                    Err(LinkError::Storage(_)) => return STORAGE_ERROR.into(),
                    Err(LinkError::Api(crate::solved_ac::ApiError::RateLimited { .. })) => return "solved.ac 요청 제한으로 토큰 갱신을 확인하지 못했습니다. 잠시 후 /check를 다시 실행하고, 재연결 안내가 나오면 /connect를 진행해주세요.".into(),
                    Err(_) => return "계정 인증을 갱신하지 못했습니다. /connect로 다시 연결해주세요. 기존 체크 기록은 유지됩니다.".into(),
                };
                match self.verifications.check(user, now, &OAuthSolves { api, tokens, account_id: account.id }) {
                    Ok(report) => {
                        let manual = self.checkins.progress(user, now).ok().flatten().map(|p| format!("직접 체크: {}/{} (별도 기록)", p.count(), p.total())).unwrap_or_default();
                        format!("{}\n\n{}\n연결 계정의 채널 알림은 실제 풀이 조회 기준이며, 확인 대기는 알림을 보류합니다.", describe_report(&report), manual)
                    },
                    Err(VerifyError::NoRecommendation) => "먼저 /today로 오늘 추천 문제를 받아주세요.".into(),
                    Err(VerifyError::WrongAccount) => "오늘 추천을 받은 핸들과 OAuth 연결 계정이 다릅니다. /account로 계정을 확인하고 오늘 추천 핸들의 계정으로 /connect를 진행해주세요. 기존 추천과 직접 체크는 바꾸지 않았습니다.".into(),
                    Err(VerifyError::Api(crate::solved_ac::ApiError::Permission)) => "solved.ac 앱 인증 또는 사용자 권한이 거부됐습니다. 서버 앱 설정을 확인하고 /connect에서 풀이 조회 권한을 다시 승인해주세요.".into(),
                    Err(VerifyError::Api(_)) => "계정 소유 확인에 실패했습니다. 잠시 후 다시 시도해주세요. 기존 기록은 유지됩니다.".into(),
                    Err(VerifyError::Storage(_)) => STORAGE_ERROR.into(),
                }
            }
            "start" => match self.settings.read(user) {
                Ok(None) => "**코테-마타-봇 시작하기**\n내 수준에 맞는 연습 문제를 매일 추천받으세요.\n\n1. solved.ac 프로필에서 본인의 아이디(핸들)를 확인하세요. Discord 닉네임과는 다릅니다.\n2. `/setup`을 선택하고 `handle`에 그 아이디, `count`에 하루 문제 수를 넣으세요. 처음에는 1~3개를 추천합니다.\n3. `/today`로 추천을 받고, 풀었으면 `/done`으로 체크하세요.\n\n추천 문제는 이미 푼 문제일 수도 있습니다. 지금은 별도의 계정 인증 없이 시작할 수 있습니다.".into(),
                Ok(Some(settings)) => match self.pause.current(user, now) {
                    Ok(PauseStatus::Active) => format!("**설정이 완료되어 있습니다!**\n{}님 · 하루 {}문제\n\n**다음: `/today`로 오늘의 추천을 받으세요.**\n문제 링크를 열어 풀고 `/done`으로 완료 체크하세요. 일부만 풀었다면 `/done problem:1`, 현황은 `/status`입니다. 같은 날에는 같은 추천이 표시됩니다.\n\n문제 수 변경: `/settings count:2`\n쉬기: `/skip days:1`\n전체 사용법: `/help`\n\n계정을 연결했다면 /check로 실제 풀이 상태를 조회하세요. 계정을 연결하면 다음 새 추천부터 해결된 문제를 제외합니다. 연결 계정은 알림 시각 이후 풀이를 자동 확인합니다.", settings.solved_ac_handle, settings.daily_count),
                    Ok(_) => "**지금은 학습을 쉬고 있습니다.**\n다시 시작하려면 `/resume` → `/today` 순서로 실행하세요.\n내 설정은 `/settings`에서 확인할 수 있습니다.".into(),
                    Err(_) => STORAGE_ERROR.into(),
                },
                Err(_) => STORAGE_ERROR.into(),
            },
            "done" => {
                let index = match integer(options, "problem").map(usize::try_from).transpose() {
                    Ok(index) => index,
                    Err(_) => return "문제 번호는 /today에 표시된 1부터 시작하는 번호를 입력하세요.".into(),
                };
                let undo = options.iter().find(|option| option.name == "undo").and_then(|option| option.value.as_bool()).unwrap_or(false);
                match self.checkins.mark(user, now, index, undo) {
                    Ok(progress) => format!("{}\n{}", if undo { "완료 체크를 취소했습니다." } else { "직접 완료 체크를 저장했습니다!" }, describe_progress(&progress)),
                    Err(CheckinError::NoRecommendation) => "먼저 /today로 오늘의 추천을 받아주세요. 추천을 받은 뒤 /done으로 완료 체크할 수 있습니다.".into(),
                    Err(CheckinError::Paused) => "학습 정지 중입니다. /resume으로 재개한 뒤 체크해주세요.".into(),
                    Err(CheckinError::InvalidIndex) => "오늘 추천에 없는 번호입니다. /today의 문제 번호를 확인하세요.".into(),
                    Err(CheckinError::Storage(_)) => STORAGE_ERROR.into(),
                }
            }
            "status" => match self.checkins.progress(user, now) {
                Ok(Some(progress)) => match self.verifications.read(user, now) {
                    Ok(Some(report)) => format!("{}\n\n{}\n최신 풀이 상태는 /check로 다시 조회하세요.", describe_progress(&progress), describe_report(&report)),
                    Ok(None) => format!("{}\n\n계정을 연결했다면 /check로 실제 풀이 상태를 확인할 수 있습니다.", describe_progress(&progress)),
                    Err(_) => format!("{}\n\n저장된 API 확인 결과를 읽지 못했습니다.", describe_progress(&progress)),
                },
                Ok(None) => "아직 오늘 추천이 없습니다. /today로 문제를 받아주세요. 오늘 추천을 받지 않은 사람에게는 알림을 보내지 않습니다.".into(),
                Err(_) => STORAGE_ERROR.into(),
            },
            "ping" => "퐁! 코테-마타-봇이 연결되어 있어요.".into(),
            "help" => String::from(
                "**코테-마타-봇 사용 안내**\n\
                 처음이라면 `/start`를 실행하세요.\n\n\
                 **매일 사용하는 명령**\n\
                 `/setup handle:내핸들 count:3` — 핸들 조회 후 하루 문제 수 저장\n\
                 `/today` — 오늘 추천 문제와 풀이할 사이트 링크\n\
                 `/done` — 오늘 추천 전체를 직접 완료 체크\n\
                 `/done problem:1` — 1번만 체크 · `undo:true`로 취소\n\
                 `/status` — 직접 체크와 저장된 API 조회 결과\n\
                 `/check` — 연결 계정의 실제 풀이 상태 조회\n\
                 `/settings` — 내 설정 조회\n\
                 `/settings count:2` — 하루 문제 수 변경\n\
                 `/skip days:1` — 1~7일 쉬기\n\
                 `/resume` — 학습 재개\n\
                 \n**선택 기능**\n\
                 `/curriculum` — 트랙 조회·선택 (캐시 구현/코딩면접/일반)\n\
                 `/goal text:LRU를 배우고 싶어요` — AI 트랙 제안 후 직접 확인\n\
                 `/hint problem:1` — 선수 개념·쉬운 연습 후보 안내\n\
                 `/settings reminder:21:00` — 미체크 채널 알림 시각 (한국 시간, 기본 21:00)\n\
                 `/connect`, `/account` — /check를 위한 계정 인증·연결 확인\n\n\
                 오늘 추천을 받은 뒤 연결 계정은 미완료가 확인된 경우, 미연결 계정은 체크가 남은 경우 설정 시각 이후 하루 한 번 멘션합니다. 쉬는 사람과 확인 대기는 제외합니다.\n\
                 /done은 자기 신고, /check는 API 조회입니다. 연결 계정 알림은 실제 풀이 조회, 미연결 계정 알림은 /done 기준입니다. 연결된 계정은 새 추천에서 해결·상태 미확정 문제를 제외합니다. 알림 전에 풀이를 자동 확인합니다. AI는 제목·태그 기반 학습 제안이며 장애 시 기본 안내로 전환합니다. DM과 추천 슬롯 교체는 아직 미지원입니다.",
            ),
            "connect" => {
                let Some(api) = &self.api else {
                    return API_REQUIRED.into();
                };
                let Some(redirect) = &self.redirect else {
                    return "관리자가 SOLVED_AC_REDIRECT_URI와 콜백 서버를 설정해야 합니다.".into();
                };
                match self.links.begin(user, redirect, api) {
                    Ok(url) => format!(
                        "[solved.ac 계정 연결하기]({url})\n10분 안에 인증해주세요. 본인에게만 보이는 링크이며 다른 사람에게 전달하지 마세요. 봇이 재시작되면 /connect를 다시 실행하세요."
                    ),
                    Err(_) => "연결 링크 생성에 실패했습니다. 잠시 후 다시 시도하세요.".into(),
                }
            }
            "account" => match self.links.account(user) {
                Ok(account) => format!(
                    "연결된 solved.ac 핸들: {}\n계정 ID: {}\n다른 계정으로 연결하려면 /connect를 실행하세요.",
                    account.handle, account.id
                ),
                Err(_) => "연결된 계정을 읽을 수 없습니다. /connect로 계정을 연결해주세요.".into(),
            },
            "setup" => {
                let Some(api) = &self.api else {
                    return API_REQUIRED.into();
                };
                let Some(handle) = string(options, "handle") else {
                    return "solved.ac 핸들을 입력하세요.".into();
                };
                let count = integer(options, "count").unwrap_or(1);
                let Ok(count) = u8::try_from(count) else {
                    return "하루 문제 수는 1~10개입니다.".into();
                };
                match self.settings.setup(user, handle, count, api) {
                    Ok(settings) => format!(
                        "설정 완료!\n{}\n/today로 추천을 받으세요. 계정이 연결돼 있으면 다음 새 추천부터 해결된 문제를 제외합니다.",
                        describe(&settings)
                    ),
                    Err(error) => settings_error(error),
                }
            }
            "settings" => {
                let count = integer(options, "count");
                let reminder = string(options, "reminder");
                if count.is_none() && reminder.is_none() {
                    return match self.settings.read(user) {
                        Ok(Some(settings)) => describe(&settings),
                        Ok(None) => "먼저 /setup으로 초기 설정을 해주세요.".into(),
                        Err(_) => STORAGE_ERROR.into(),
                    };
                }
                // Updating count/time does not perform a profile lookup.
                let daily_count = match count.map(u8::try_from).transpose() {
                    Ok(value) => value,
                    Err(_) => return "하루 문제 수는 1~10개입니다.".into(),
                };
                match self.settings.update(
                    user,
                    SettingsUpdate {
                        daily_count,
                        reminder_time: reminder,
                        ..Default::default()
                    },
                    &NoProfileLookup,
                ) {
                    Ok(settings) => format!(
                        "설정 변경 완료.\n{}\n설정 시각 이후 연결 계정은 실제 미완료 확인 시, 미연결 계정은 미체크 시 이 채널에 하루 한 번 알립니다. 확인 대기는 보류하며 DM은 보내지 않습니다.",
                        describe(&settings)
                    ),
                    Err(error) => settings_error(error),
                }
            }
            "skip" => {
                let days = integer(options, "days").unwrap_or(1);
                let Ok(days) = u8::try_from(days) else {
                    return "스킵 기간은 1~7일입니다.".into();
                };
                match self.pause.skip(user, now, Some(days)) {
                    Ok(PauseStatus::SkippedUntil(date)) => format!(
                        "한국 시간 {date} 전까지 학습을 쉽니다. /resume으로 언제든 재개할 수 있습니다. 기간이 끝나도 재개 전까지 정지 상태를 유지합니다."
                    ),
                    Ok(_) => {
                        "학습 정지 상태를 저장했습니다. /resume으로 재개할 수 있습니다.".into()
                    }
                    Err(PauseError::InvalidDays) => "스킵 기간은 1~7일입니다.".into(),
                    Err(PauseError::Storage(_)) => STORAGE_ERROR.into(),
                }
            }
            "resume" => match self.pause.resume(user) {
                Ok(()) => "학습 정지를 해제했습니다.".into(),
                Err(_) => STORAGE_ERROR.into(),
            },
            "curriculum" => {
                if let Some(track) = string(options, "track")
                    && self
                        .curriculum
                        .select(user, (track != "tier").then_some(track))
                        .is_err()
                {
                    return "트랙 저장 실패. cache, coding-interview, tier 중 선택해주세요.".into();
                }
                match self.curriculum.plan(user) {
                    Ok(Plan::General) => "일반 티어 추천 모드입니다. 트랙: cache(캐시 구현), coding-interview(코딩면접), tier(일반). /today로 추천을 조회하세요. 변경한 트랙은 다음 신규 추천부터 적용됩니다.".into(),
                    Ok(Plan::Learning { key, stage }) => format!("트랙: {}\n현재 단계: {}\n완료 기준: 해당 단계 문제 {}개 풀이 확인\n/today에서 단계 태그에 맞는 문제를 추천합니다. 변경은 다음 신규 추천부터 적용됩니다. 풀이 상태는 /check에서 확인할 수 있으며 자동 단계 진행은 아직 연결 중입니다.", key.track, stage.concept, stage.required),
                    Ok(Plan::Completed { track }) => format!("{track} 트랙 완료! 다른 트랙 또는 tier를 선택하세요."),
                    Err(_) => STORAGE_ERROR.into(),
                }
            }
            "today" => {
                let Some(api) = &self.api else {
                    return API_REQUIRED.into();
                };
                match self.today_recommendation(user, now, api) {
                    Ok(recommendation) => {
                        let checked = match self.checkins.progress(user, now) {
                            Ok(Some(progress)) => format!("직접 완료 체크: {}/{}", progress.count(), progress.total()),
                            _ => "체크 현황은 /status에서 확인하세요.".into(),
                        };
                        format!("{}\n\n{}\n풀었으면 /done · 일부만 풀었으면 /done problem:1\n설정한 알림 시각 이후 연결 계정은 실제 미완료 확인 시, 미연결 계정은 미체크 시 하루 한 번 멘션합니다. 시각 변경은 /settings reminder:21:00", describe_recommendation(&recommendation), checked)
                    },
                    Err(message) => message,
                }
            }
            _ => "알 수 없는 명령입니다. /help를 확인해주세요.".into(),
        }
    }
}

struct NoProfileLookup;
impl ProfileLookup for NoProfileLookup {
    fn lookup(&self, _: &str) -> Result<Profile, LookupError> {
        Err(LookupError::Unavailable)
    }
}

fn describe(settings: &Settings) -> String {
    format!(
        "핸들: {}\n티어: {}\n하루 문제 수: {}\n알림 시각: {} (한국 시간)",
        settings.solved_ac_handle, settings.tier, settings.daily_count, settings.reminder_time
    )
}

fn describe_report(report: &Report) -> String {
    let solved = report
        .results
        .iter()
        .filter(|row| row.status == VerifiedStatus::Solved)
        .count();
    let pending = report
        .results
        .iter()
        .filter(|row| row.status == VerifiedStatus::Pending)
        .count();
    let checked_at = report
        .checked_at
        .with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).expect("Seoul offset"));
    let mut lines = vec![format!(
        "**실제 풀이 조회: {}/{} 해결 · {}개 확인 대기**\n대상: {} · {} 추천\n조회 시각: {} (한국 시간)",
        solved,
        report.results.len(),
        pending,
        report.handle,
        report.date,
        checked_at.format("%Y-%m-%d %H:%M:%S")
    )];
    for (index, row) in report.results.iter().enumerate() {
        let label = match row.status {
            VerifiedStatus::Solved => "✅ 해결 확인",
            VerifiedStatus::Unsolved => "⬜ 미완료 확인",
            VerifiedStatus::Pending => match row.reason {
                Some(PendingReason::Permission) => "❔ 권한 확인 필요 (/connect)",
                Some(PendingReason::RateLimited) => "❔ 요청 제한 · 나중에 재조회",
                Some(PendingReason::Unavailable) => "❔ 외부 조회 실패",
                Some(PendingReason::NotFound) => "❔ 문제 상세를 찾지 못함",
                Some(PendingReason::InvalidResponse) => "❔ 응답 형식 확인 필요",
                _ => "❔ 상태 미제공 · 풀이 여부 미확정",
            },
        };
        lines.push(format!("{}. {} — {}", index + 1, row.problem.title, label));
        if row.status == VerifiedStatus::Pending
            && let Some(previous) = row.last_confirmed_solved_at
        {
            lines.push(format!(
                "   이전 해결 확인: {} UTC (이번 조회에서는 미확정)",
                previous.format("%Y-%m-%d %H:%M:%S")
            ));
        }
    }
    lines.push("solved.ac에 반영된 해결 상태 기준입니다. 오늘 새로 제출했는지까지 증명하지는 않습니다. 확인 대기는 미완료로 단정하지 않습니다.".into());
    lines.join("\n")
}

fn describe_progress(progress: &Progress) -> String {
    let mut lines = vec![format!(
        "**오늘 직접 완료 체크: {}/{}**",
        progress.count(),
        progress.total()
    )];
    for (index, problem) in progress.recommendation.problems.iter().enumerate() {
        lines.push(format!(
            "{} {}. {}",
            if progress.checked.contains(&problem.id) {
                "✅"
            } else {
                "⬜"
            },
            index + 1,
            problem.title
        ));
    }
    lines.push(if progress.count() == progress.total() {
        "오늘 추천을 모두 직접 체크했습니다. 연결 계정의 알림 여부는 실제 풀이 조회 결과로 결정됩니다."
    } else {
        "/done으로 전체 체크 · /done problem:1로 한 문제 체크 · /done problem:1 undo:true로 취소"
    }.into());
    lines.push(
        "직접 체크한 기록이며 실제 저지 풀이 확인이나 커리큘럼 완료 실적에 반영하지 않습니다."
            .into(),
    );
    lines.join("\n")
}

fn describe_recommendation(recommendation: &Recommendation) -> String {
    let mut lines = vec![format!(
        "**오늘의 추천 문제** · {} · 현재 티어 {}",
        recommendation.handle, recommendation.tier
    )];
    if let Some(topic) = &recommendation.topic {
        lines.push(format!("학습 주제: {topic}"));
    }
    lines.push(if recommendation.verified_account_id.is_some() {
        "추천 시점에 연결 계정의 미완료 상태를 확인한 문제입니다. 현재 상태는 /check로 다시 확인하세요. 같은 날에는 저장된 추천을 보여줍니다."
    } else {
        "추천 시점에 풀이 여부를 확인하지 않았으므로 이미 푼 문제가 포함될 수 있습니다. 같은 날에는 저장된 추천을 보여줍니다."
    }.into());
    let count = recommendation.problems.len();
    for (index, problem) in recommendation.problems.iter().enumerate() {
        let line = format!(
            "{}. {}\n<{}>",
            index + 1,
            problem.title,
            problem.submission_url
        );
        lines.push(line);
    }
    if count < usize::from(recommendation.requested) {
        lines.push(format!(
            "요청 {}개 중 {}개 추천: 조회한 후보에서 조건에 맞는 문제가 부족합니다.",
            recommendation.requested, count
        ));
    }
    lines.join("\n")
}

fn settings_error(error: SettingsError) -> String {
    match error {
        SettingsError::InvalidCount => "하루 문제 수는 1~10개입니다.".into(),
        SettingsError::InvalidHandle => "핸들을 입력해주세요.".into(),
        SettingsError::InvalidReminderTime => "알림 시각은 00:00~23:59 형식으로 입력하세요.".into(),
        SettingsError::SetupRequired => "먼저 /setup으로 초기 설정을 해주세요.".into(),
        SettingsError::Lookup(LookupError::UnknownHandle) => "존재하지 않는 solved.ac 핸들입니다.".into(),
        SettingsError::Lookup(LookupError::Unavailable) => "solved.ac 조회에 실패했습니다. 기존 설정은 유지됩니다. 앱 인증값과 API 연결 상태를 확인해주세요.".into(),
        SettingsError::InvalidGoal => "올바른 목표를 입력해주세요.".into(),
        SettingsError::Storage(_) => STORAGE_ERROR.into(),
    }
}

fn string<'a>(options: &'a [CommandDataOption], name: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|option| option.name == name)?
        .value
        .as_str()
}

fn integer(options: &[CommandDataOption], name: &str) -> Option<i64> {
    options
        .iter()
        .find(|option| option.name == name)?
        .value
        .as_i64()
}

pub fn definitions() -> Vec<CreateCommand> {
    let count = || {
        CreateCommandOption::new(CommandOptionType::Integer, "count", "하루 문제 수 (1~10)")
            .min_int_value(1)
            .max_int_value(10)
    };
    vec![
        CreateCommand::new("ping").description("봇 연결 확인"),
        CreateCommand::new("help").description("사용 가능한 명령과 연결 상태 안내"),
        CreateCommand::new("start").description("처음 시작하기 · 내 설정에 맞는 다음 단계 안내"),
        CreateCommand::new("done")
            .description("오늘 추천을 직접 완료 체크 (옵션 없으면 전체)")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "problem",
                    "/today의 문제 번호 (생략하면 전체)",
                )
                .min_int_value(1)
                .max_int_value(10),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "undo",
                "true면 선택한 완료 체크 취소",
            )),
        CreateCommand::new("status").description("오늘의 직접 완료 체크 현황"),
        CreateCommand::new("check").description("OAuth 연결 계정으로 오늘 추천 문제 풀이 조회"),
        CreateCommand::new("goal")
            .description("배우고 싶은 목표를 AI로 지원 트랙에 매칭 (제안만)")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "text",
                    "예: LRU와 LFU를 배우고 싶어요",
                )
                .required(true)
                .max_length(500),
            ),
        CreateCommand::new("hint")
            .description("오늘 추천 문제의 선수 개념과 쉬운 연습 후보 안내")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::Integer,
                    "problem",
                    "/today의 문제 번호 (기본 1번)",
                )
                .min_int_value(1)
                .max_int_value(10),
            ),
        CreateCommand::new("connect").description("내 solved.ac 계정 OAuth 연결"),
        CreateCommand::new("account").description("연결된 solved.ac 계정 조회"),
        CreateCommand::new("setup")
            .description("solved.ac 핸들과 하루 문제 수 설정")
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "handle", "내 solved.ac 핸들")
                    .required(true)
                    .max_length(50),
            )
            .add_option(count()),
        CreateCommand::new("settings")
            .description("내 설정 조회 및 변경")
            .add_option(count())
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "reminder",
                    "미체크 채널 알림 시각 · 한국 시간 HH:MM",
                )
                .max_length(5),
            ),
        CreateCommand::new("skip")
            .description("학습을 1~7일 정지")
            .add_option(
                CreateCommandOption::new(CommandOptionType::Integer, "days", "쉴 일수 (기본 1일)")
                    .min_int_value(1)
                    .max_int_value(7),
            ),
        CreateCommand::new("resume").description("학습 정지 해제"),
        CreateCommand::new("curriculum")
            .description("학습 트랙 선택 및 진행 조회")
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "track", "직접 선택할 트랙")
                    .add_string_choice("일반 티어 추천", "tier")
                    .add_string_choice("LRU·LFU 캐시 구현", "cache")
                    .add_string_choice("코딩 기술면접", "coding-interview"),
            ),
        CreateCommand::new("today").description("오늘의 티어·주제별 추천 (풀이 여부 미확인)"),
    ]
}

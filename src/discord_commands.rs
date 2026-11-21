use crate::account_link::AccountLinks;
use crate::curriculum::{CurriculumStore, Plan};
use crate::pause::{PauseError, PauseStatus, PauseStore};
use crate::recommendation::{Recommendation, RecommendationStore};
use crate::settings::{
    LookupError, Profile, ProfileLookup, Settings, SettingsError, SettingsStore, SettingsUpdate,
};
use crate::solved_ac::SolvedAc;
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
        }
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

    pub fn execute(
        &self,
        user: u64,
        name: &str,
        options: &[CommandDataOption],
        now: DateTime<Utc>,
    ) -> String {
        match name {
            "ping" => "퐁! 코테-마타-봇이 연결되어 있어요.".into(),
            "help" => format!(
                "**코테-마타-봇 사용 안내**\n\
                 `/setup handle:내핸들 count:3` — 핸들 조회 후 하루 문제 수 저장\n\
                 `/connect` — 내 solved.ac 계정 OAuth 연결\n\
                 `/account` — 연결된 계정 조회\n\
                 `/settings` — 내 설정 조회\n\
                 `/settings count:2` — 하루 문제 수 변경\n\
                 `/settings reminder:21:00` — 알림 시각 저장 (한국 시간)\n\
                 `/curriculum` — 커리큘럼 조회\n\
                 `/curriculum track:cache` — 캐시 구현 트랙 선택\n\
                 `/skip days:1` — 1~7일 쉬기\n\
                 `/resume` — 학습 재개\n\
                 `/today` — 현재 티어·트랙에 맞는 오늘의 추천 문제\n\n\
                 **현재 연결 상태**\nsolved.ac 앱: {}\n\
                 `/today`는 OAuth 없이 앱 인증으로 조회한 추천입니다. 이미 푼 문제 여부는 확인하지 않습니다. 풀이 확인·자동 DM·AI 도움은 아직 연결 중입니다.",
                if self.api.is_some() {
                    "설정됨 (실호출은 /setup으로 확인)"
                } else {
                    "미설정"
                }
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
                        "설정 완료!\n{}\n/today로 오늘의 추천을 받으세요. 이미 푼 문제 여부는 확인하지 않습니다.",
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
                        "설정 변경 완료.\n{}\n자동 DM은 아직 연결 중입니다.",
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
                    Ok(Plan::Learning { key, stage }) => format!("트랙: {}\n현재 단계: {}\n완료 기준: 해당 단계 문제 {}개 풀이 확인\n/today에서 단계 태그에 맞는 문제를 추천합니다. 변경은 다음 신규 추천부터 적용됩니다. 풀이 확인과 자동 단계 진행은 아직 연결 중입니다.", key.track, stage.concept, stage.required),
                    Ok(Plan::Completed { track }) => format!("{track} 트랙 완료! 다른 트랙 또는 tier를 선택하세요."),
                    Err(_) => STORAGE_ERROR.into(),
                }
            }
            "today" => {
                let Some(api) = &self.api else {
                    return API_REQUIRED.into();
                };
                match self.recommendations.today(user, now, &self.settings, api) {
                    Ok(recommendation) => describe_recommendation(&recommendation),
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

fn describe_recommendation(recommendation: &Recommendation) -> String {
    let mut lines = vec![format!(
        "**오늘의 추천 문제** · {} · 현재 티어 {}",
        recommendation.handle, recommendation.tier
    )];
    if let Some(topic) = &recommendation.topic {
        lines.push(format!("학습 주제: {topic}"));
    }
    lines.push("OAuth 미사용: 이미 푼 문제 여부는 확인하지 않았습니다. 같은 날에는 저장된 추천을 보여줍니다.".into());
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
                    "한국 시간 HH:MM (자동 DM은 연결 중)",
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

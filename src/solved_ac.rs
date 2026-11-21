use crate::settings::{LookupError, Profile, ProfileLookup};
use crate::{
    assignment::Problem,
    recommendation::{Candidate, RecommendationSource},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{io, time::Duration};
use url::Url;

const BASE: &str = "https://solved.ac";

pub struct SolvedAc {
    http: ureq::Agent,
    client_id: String,
    client_secret: String,
}

pub struct Authorization {
    pub url: Url,
    state: String,
    verifier: String,
    redirect_uri: String,
    expires: std::time::Instant,
}

impl Authorization {
    pub(crate) fn matches(&self, state: &str) -> bool {
        state == self.state && std::time::Instant::now() < self.expires
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Account {
    pub id: u64,
    pub handle: String,
}

#[derive(Deserialize, Serialize)]
pub struct Tokens {
    pub access_token: String,
    pub expires_in: u64,
    pub scope: String,
    pub refresh_token: Option<String>,
}

impl Tokens {
    pub fn permits_solves(&self) -> bool {
        self.scope
            .split_ascii_whitespace()
            .any(|scope| scope == "solves")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    InvalidState,
    Permission,
    NotFound,
    RateLimited { retry_after_seconds: Option<u64> },
    Rejected(u16),
    Unavailable,
    InvalidResponse,
}

impl SolvedAc {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            http: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(15))
                .redirects(0)
                .build(),
            client_id,
            client_secret,
        }
    }

    pub fn begin(&self, registered_redirect: &str) -> io::Result<Authorization> {
        let state = random_token()?;
        let verifier = random_token()?;
        let hash = digest::digest(&digest::SHA256, verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hash.as_ref());
        let mut url = Url::parse(BASE).expect("valid solved.ac base URL");
        url.set_path("/oauth/authorize");
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", &self.client_id),
            ("redirect_uri", registered_redirect),
            ("scope", "solves offline.access"),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ]);
        Ok(Authorization {
            url,
            state,
            verifier,
            redirect_uri: registered_redirect.into(),
            expires: std::time::Instant::now() + Duration::from_secs(600),
        })
    }

    pub fn exchange(
        &self,
        pending: Authorization,
        state: &str,
        code: &str,
    ) -> Result<Tokens, ApiError> {
        if !pending.matches(state) || code.is_empty() {
            return Err(ApiError::InvalidState);
        }
        self.token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &pending.redirect_uri),
            ("code_verifier", &pending.verifier),
        ])
    }

    /// Call once per token and persist the replacement before using it again.
    pub fn refresh(&self, refresh_token: &str) -> Result<Tokens, ApiError> {
        self.token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
    }

    pub fn account(&self, tokens: &Tokens) -> Result<Account, ApiError> {
        decode(self.request("GET", &endpoint(&["me"]), Some(tokens)).call())
    }

    pub fn solved(&self, tokens: &Tokens, problem_id: u64, sub: &str) -> Result<bool, ApiError> {
        if !tokens.permits_solves() {
            return Err(ApiError::Permission);
        }
        let mut url = endpoint(&["problems", &problem_id.to_string()]);
        if !sub.is_empty() {
            url.query_pairs_mut().append_pair("sub", sub);
        }
        let body: serde_json::Value = decode(self.request("GET", &url, Some(tokens)).call())?;
        match body.get("userStatus") {
            Some(serde_json::Value::String(status)) if status == "solved" => Ok(true),
            Some(serde_json::Value::String(status)) if status == "partial" || status == "tried" => {
                Ok(false)
            }
            // A null response still needs real-account validation against anonymous responses.
            Some(serde_json::Value::Null) => Err(ApiError::Permission),
            _ => Err(ApiError::InvalidResponse),
        }
    }

    fn token(&self, fields: &[(&str, &str)]) -> Result<Tokens, ApiError> {
        decode(
            self.request("POST", &endpoint(&["oauth", "token"]), None)
                .send_form(fields),
        )
    }

    fn request(&self, method: &str, url: &Url, tokens: Option<&Tokens>) -> ureq::Request {
        let request = self
            .http
            .request(method, url.as_str())
            .set("x-client-id", &self.client_id)
            .set("x-client-secret", &self.client_secret);
        match tokens {
            Some(tokens) => {
                request.set("Authorization", &format!("Bearer {}", tokens.access_token))
            }
            None => request,
        }
    }
}

impl ProfileLookup for SolvedAc {
    fn lookup(&self, handle: &str) -> Result<Profile, LookupError> {
        #[derive(Deserialize)]
        struct PublicProfile {
            handle: String,
            tier: Option<u8>,
        }
        let profile: PublicProfile = decode(
            self.request("GET", &endpoint(&["users", handle]), None)
                .call(),
        )
        .map_err(|error| {
            if error == ApiError::NotFound {
                LookupError::UnknownHandle
            } else {
                LookupError::Unavailable
            }
        })?;
        Ok(Profile {
            handle: profile.handle,
            tier: profile
                .tier
                .map_or_else(|| "unrated".into(), |tier| tier.to_string()),
        })
    }
}

impl RecommendationSource for SolvedAc {
    fn problems(&self) -> Result<Vec<Candidate>, String> {
        let mut candidates = Vec::new();
        let mut received = 0;
        // Do not assume v3's search grammar works in v4. Filter levels and stage tags
        // locally, using a bounded set of public list pages. No solve-state claims.
        for page in 1..=5 {
            let mut url = endpoint(&["problems"]);
            url.query_pairs_mut().append_pair("page", &page.to_string());
            let body: serde_json::Value =
                decode(self.request("GET", &url, None).call()).map_err(recommendation_api_error)?;
            let items = list_items(&body).ok_or(
                "solved.ac 문제 목록 응답 형식을 읽지 못했습니다. 실응답 형식 확인이 필요합니다.",
            )?;
            received += items.len();
            for item in items {
                if let Some(candidate) = parse_candidate(item) {
                    candidates.push(candidate);
                }
            }
            if items.len() < 50 {
                break;
            }
        }
        if received > 0 && candidates.is_empty() {
            return Err("목록은 받았지만 난이도·제목·HTTPS 제출 링크가 있는 후보를 읽지 못했습니다. 관리자에게 check-solved-ac.py로 응답 형식을 확인해달라고 요청하세요.".into());
        }
        Ok(candidates)
    }
}

fn recommendation_api_error(error: ApiError) -> String {
    match error {
        ApiError::Permission => {
            "solved.ac 앱 인증이 거부됐습니다. 서버 Client ID와 Client Secret을 확인하세요.".into()
        }
        ApiError::RateLimited {
            retry_after_seconds: Some(seconds),
        } => format!("solved.ac 요청 제한입니다. {seconds}초 후 다시 시도하세요."),
        ApiError::RateLimited { .. } => {
            "solved.ac 요청 제한입니다. 잠시 후 다시 시도하세요.".into()
        }
        ApiError::InvalidResponse => "solved.ac 응답 형식을 읽지 못했습니다.".into(),
        _ => "solved.ac 문제 조회에 실패했습니다. 잠시 후 다시 시도하세요.".into(),
    }
}

fn list_items(body: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    body.as_array().or_else(|| body.get("items")?.as_array())
}

fn parse_candidate(item: &serde_json::Value) -> Option<Candidate> {
    let id = item.get("problemId").or_else(|| item.get("id"))?.as_u64()?;
    let level = item.get("level").or_else(|| item.get("tier"))?.as_u64()?;
    if id == 0 || !(1..=30).contains(&level) {
        return None;
    }
    let title = item
        .get("title")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            item.get("titles")?
                .as_array()?
                .iter()
                .find_map(|title| title.get("title")?.as_str())
        })?;
    if title.trim().is_empty() {
        return None;
    }
    let submission_url = item.get("ojLinks")?.as_array()?.iter().find_map(|link| {
        let raw = link.as_str().or_else(|| link.get("url")?.as_str())?;
        let url = Url::parse(raw).ok()?;
        (url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none())
        .then(|| url.to_string())
    })?;
    let tags = item
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.as_str().or_else(|| tag.get("key")?.as_str()))
        .map(str::to_owned)
        .collect();
    Some(Candidate {
        problem: Problem {
            id: id.to_string(),
            title: title.chars().take(160).collect(),
            submission_url,
        },
        level: level as u8,
        tags,
    })
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| io::Error::other("secure random source unavailable"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn endpoint(segments: &[&str]) -> Url {
    let mut url = Url::parse(BASE).expect("valid solved.ac base URL");
    url.path_segments_mut()
        .expect("hierarchical URL")
        .extend(["api", "v4"])
        .extend(segments.iter().copied());
    url
}

fn decode<T: serde::de::DeserializeOwned>(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<T, ApiError> {
    match result {
        Ok(response) if (200..300).contains(&response.status()) => {
            response.into_json().map_err(|_| ApiError::InvalidResponse)
        }
        Ok(response) => Err(ApiError::Rejected(response.status())),
        Err(ureq::Error::Status(401 | 403, _)) => Err(ApiError::Permission),
        Err(ureq::Error::Status(404, _)) => Err(ApiError::NotFound),
        Err(ureq::Error::Status(429, response)) => Err(ApiError::RateLimited {
            retry_after_seconds: response
                .header("Retry-After")
                .and_then(|value| value.parse().ok()),
        }),
        Err(ureq::Error::Status(status, _)) => Err(ApiError::Rejected(status)),
        Err(ureq::Error::Transport(_)) => Err(ApiError::Unavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_parser_requires_real_https_links_and_valid_levels() {
        let valid = serde_json::json!({"problemId":123,"level":10,"title":"문제", "tags":[{"key":"dp"}],"ojLinks":[{"url":"https://judge.example/problems/7"}]});
        let candidate = parse_candidate(&valid).unwrap();
        assert_eq!(candidate.problem.id, "123");
        assert_eq!(
            candidate.problem.submission_url,
            "https://judge.example/problems/7"
        );
        assert_eq!(candidate.tags, ["dp"]);
        for (field, value) in [
            ("ojLinks", serde_json::json!([])),
            (
                "ojLinks",
                serde_json::json!([{"url":"javascript:alert(1)"}]),
            ),
            ("level", serde_json::json!(99)),
            ("title", serde_json::json!("")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(parse_candidate(&invalid).is_none());
        }
        assert!(list_items(&serde_json::json!({"error":"bad request"})).is_none());
        assert_eq!(
            list_items(&serde_json::json!({"items":[]})).unwrap().len(),
            0
        );
    }

    #[test]
    fn oauth_requests_user_scopes_and_validates_state_before_exchange() {
        let client = SolvedAc::new("app".into(), "secret".into());
        let redirect = "https://bot.example/oauth/callback";
        let pending = client.begin(redirect).unwrap();
        let params: std::collections::BTreeMap<_, _> =
            pending.url.query_pairs().into_owned().collect();
        assert_eq!(pending.url.path(), "/oauth/authorize");
        assert_eq!(params["redirect_uri"], redirect);
        assert_eq!(params["scope"], "solves offline.access");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["code_challenge"].len(), 43);
        assert_eq!(params["state"], pending.state);
        assert!(matches!(
            client.exchange(pending, "wrong-state", "code"),
            Err(ApiError::InvalidState)
        ));
        assert_eq!(
            endpoint(&["problems", "1000"]).path(),
            "/api/v4/problems/1000"
        );
    }

    #[test]
    fn missing_solves_scope_never_queries_problem_status() {
        let client = SolvedAc::new("app".into(), "secret".into());
        let tokens = Tokens {
            access_token: String::new(),
            expires_in: 3600,
            scope: "profile offline.access".into(),
            refresh_token: None,
        };
        assert_eq!(client.solved(&tokens, 1000, ""), Err(ApiError::Permission));
    }
}

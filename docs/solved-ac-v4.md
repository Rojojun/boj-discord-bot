# solved.ac v4 연동 확인

[공식 명세](https://backstage.solved.ac/ko/developer/docs)에서 확인한 서버는 `https://solved.ac`이다.

| 경로 | 확인된 계약 |
|---|---|
| `GET /api/v4/me` | 액세스 토큰의 계정 `id`와 현재 `handle`을 반환한다. 계정 ID는 고정이고 핸들은 변경될 수 있다. 사용자 미인증은 401이다. |
| `GET /api/v4/problems/{problemId}` | 정수 문제 ID와 선택적 `sub`를 받는다. 조회자의 `userStatus`, 난이도, 태그, `ojLinks`를 반환한다. 문제가 없으면 404이다. 핸들 파라미터는 제공되지 않는다. `x-access`는 public, 사용자 문맥 스코프는 `solves`다. |
| `GET /api/v4/users/{handle}` | 앱 인증으로 `id`, `handle`, `tier`를 읽는다. `tier`는 0–31의 정수 또는 null이다. |
| `GET /api/v4/problems` | 앱 인증으로 `q`, `page`, `sort`, `direction`을 사용할 수 있다. 페이지 크기는 50이다. 목록에서 토큰으로 조회자 상태를 사용할 수 있는지는 검증 전이다. |

## 첫 실호출 검증

1. 사용자 액세스 토큰으로 `/api/v4/me`를 조회해 대상 계정인지 확인한다.
2. 같은 토큰으로 solved.ac에서 풀이 기록이 확인된 문제와 아직 풀지 않은 문제의 상세를 각각 curl로 조회한다. 저지 자체 문제 ID와 solved.ac의 `problemId`를 구분하고, 하위 문제가 있으면 `sub`도 보존한다.
3. 계정 소유·solves 권한 확인을 통과한 사용자 인증 상세 응답에서 `userStatus`가 `solved`일 때 완료, `partial`, `tried`, 명시적인 `null`일 때 미완료로 처리한다. 필드 누락·새로운 값·HTTP 오류는 확인 대기로 처리한다.

2026-11-23 홈서버 진단: profile/solves/offline.access 권한, `/me` 계정 ID 일치, 동일 토큰으로 1000·1001은 solved, 1002~1004는 필드가 존재하며 null인 응답을 확인했다. 이를 바탕으로 명시적 null을 해결 기록 없음으로 해석하도록 수정했다. 모든 null이 권한과 무관하다는 일반적 보장은 아니므로 계정·권한 검사를 유지한다.
4. OAuth 동의 요청에 `solves`를 포함하고 토큰 교환 응답의 `scope`에서 부여 여부를 확인한다. 지속 자동 확인에는 `offline.access`도 요청한다.
5. 테스트 계정에서 이전에 미해결로 확인한 문제를 제출하고, solved.ac의 상태 반영까지 시간을 확인한다.

현재 실행 환경에서 두 경로와 공개 헬스 체크를 curl로 시도했으나 DNS 해석에 실패했다. 실제 HTTP 응답은 미검증이다. 토큰과 시크릿은 저장소에 기록하지 않는다.

## 인증 계약

- 모든 요청에는 앱의 `x-client-id`와 `x-client-secret`을 사용한다. 사용자 요청은 `Authorization: Bearer <access_token>`을 추가한다.
- 사용자는 `https://solved.ac/oauth/authorize`에서 동의한다. 등록한 콜백 주소, 검증할 `state`, SHA-256 기반 PKCE 챌린지와 `code_challenge_method=S256`을 사용한다. 인증 코드는 `POST /api/v4/oauth/token`에서 교환한다.
- 액세스 토큰은 1시간 유효하다. 자동 알림에는 `offline.access`가 필요하다. 리프레시 토큰은 90일 유효하며 사용하면 새 토큰으로 교체된다. 계정별 갱신을 직렬화하고, 기존 토큰 재사용 시 해당 로그인 전체가 취소되므로 요청 결과가 불명확할 때 기존 토큰을 자동 재시도하지 않는다.
- 공개 엔드포인트도 토큰에 해당 스코프가 없으면 익명 응답을 반환할 수 있다. `/me` 성공은 토큰 계정만 확인하므로 풀이 상태 검증에서는 토큰의 스코프도 확인한다.
- 429 응답은 `Retry-After`에 맞춰 재시도한다.

## Rust 연결 지점

### 홈서버 응답 확인 (2026-11-21)

- 앱 인증으로 문제 목록 조회 HTTP 200 확인. 목록 키는 `problems`이며 `total`, `page`, `pageSize`를 함께 반환한다.
- 문제 목록 항목의 `titles`는 `language`, `title`, `isOriginal`, `latex`, `html`을 가진 배열이다. 태그 배열은 `id`, `key`, `isMeta`를 가진다. 목록에 `ojLinks`는 없다.
- 문제 상세의 `ojLinks`는 `ojId`, `ojName`, `nativeProblemId`, `url`, `logoUrl`을 가진 배열이다. 예시 응답에서 BOJ 1000의 URL을 확인했다.
- Rust 추천 경로는 목록에서 난이도·태그 후보를 추린 뒤 상세에서 HTTPS 링크를 가져온다. 같은 문제의 `subId`도 보존한다. OAuth 미사용 추천이므로 `userStatus`를 풀이 여부로 사용하지 않는다.
- 최종 `/today` 전체 흐름은 수정 배포 후 운영 검증이 필요하다.

| 확인할 API | Rust 연결 지점 |
|---|---|
| 핸들과 티어 조회 | `ProfileLookup` |
| 난이도 및 단계 태그별 문제 검색 | `ProblemLookup` |
| 배정 문제별 풀이 상태 | `SolveLookup` |
| 태그 및 쉬운 문제 후보 | `HelpLookup` |

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
3. 인증 계정의 상세 응답에서 `userStatus`가 `solved`일 때 완료, `partial`과 `tried`일 때 미완료로 처리한다. `null`이 토큰 권한 부족과 미시도 상태를 구분하는지 실호출로 검증한다. 필드 누락과 HTTP 오류는 확인 대기로 처리한다.
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

| 확인할 API | Rust 연결 지점 |
|---|---|
| 핸들과 티어 조회 | `ProfileLookup` |
| 난이도 및 단계 태그별 문제 검색 | `ProblemLookup` |
| 배정 문제별 풀이 상태 | `SolveLookup` |
| 태그 및 쉬운 문제 후보 | `HelpLookup` |

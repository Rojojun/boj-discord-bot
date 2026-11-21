# boj-discord-bot

solved.ac 티어로 매일 연습 문제를 고르고, 목표별 학습과 풀이 확인을 제공하는 3인용 Discord 봇 프로젝트다. Rust로 구현하고 사용자별 JSON 파일을 사용한다.

## 현재 상태

### Discord 연결 확인

Discord 명령과 사용자별 설정 저장을 연결했다. 프로젝트 폴더에서 다음과 같이 실행한다.

1. `.env.example`을 `.env`로 복사한다. `.env`는 Git에서 제외된다.
2. `DISCORD_BOT_TOKEN`에 Bot Token을, `DISCORD_GUILD_ID`에 초대한 테스트 서버 ID를, `DISCORD_CHANNEL_ID`에 사용할 채널 ID를 넣는다. Application ID와 채널 ID는 예시에 설정되어 있다.
3. `sh scripts/run.sh`로 실행하고 테스트 서버에서 `/ping`을 입력한다. Cargo가 PATH에 없어도 기본 Rust 설치 경로에서 찾는다. 첫 실행에는 의존성 다운로드와 빌드가 필요하다.

서버 ID는 Discord 설정 → 고급 → 개발자 모드를 켠 뒤 서버 아이콘을 우클릭해 복사한다.
토큰은 채팅이나 저장소에 올리지 않는다. 별도의 privileged intent 설정은 필요 없다.
명령은 지정한 서버에만 등록하며 지정한 채널에서만 응답한다. 응답은 명령을 실행한 사용자에게만 보인다. 다른 방의 가시성과 명령 표시 제한은 Discord 채널 권한 및 서버 연동 설정에서 별도로 설정한다. 종료는 Ctrl+C다.
`/help`에서 사용법과 연결 상태를 볼 수 있다. `/setup handle:핸들 count:3`은 solved.ac 프로필 조회 후 초기 설정을 저장한다. 서버에 `SOLVED_AC_CLIENT_ID`와 `SOLVED_AC_CLIENT_SECRET`을 모두 설정해야 한다.

`/settings`는 설정 조회, `count`와 `reminder` 옵션은 설정 변경이다. `/skip days:1`, `/resume`은 학습 정지 상태를 저장·해제한다. `/curriculum track:cache`는 트랙을 저장하고 현재 단계를 보여준다. 각 사용자는 자신의 데이터만 변경한다. 데이터는 `BOT_DATA_DIR` (기본 `data`) 아래에 저장한다.

`SOLVED_AC_REDIRECT_URI`가 설정돼 있으면 `127.0.0.1:8787/oauth/callback`에서 OAuth 콜백을 받는다. `/connect`는 본인에게만 보이는 인증 링크를 만들고 `/account`는 저장된 연결 계정을 보여준다. HTTPS reverse proxy로 콜백만 연결하고 Backstage에 동일한 Redirect URI를 등록해야 한다. 콜백 URL의 쿼리는 인증 코드가 있으므로 프록시 접근 로그에서 기록하지 않는다. 링크는 10분 동안 유효하며 재시작 시 다시 생성해야 한다.

`/today`는 OAuth 없이 앱 인증으로 문제 목록을 최대 5페이지 조회하고 현재 티어 → 한 단계 아래 → 한 단계 위 순으로 후보를 고른다. 커리큘럼을 선택했다면 해당 단계 태그로 제한한다. 같은 날에는 저장된 추천을 재사용한다. 한국 시간 자정에 새 추천을 만들며 스킵 중에는 차단한다. 신규 추천 때 프로필 티어를 다시 조회한다. 후보가 부족하면 부족분을 표시한다. 조회 범위 밖 문제는 탐색하지 않으며, 날짜가 바뀌어도 같은 문제가 다시 나올 수 있다.

이 모드는 **풀이 여부를 확인하지 않은 추천**이다. 미해결 과제나 커리큘럼 완료 실적으로 기록하지 않는다. 데이터도 `users/<id>/recommendations/`에 별도로 저장한다. 풀이 확인, 자동 DM과 AI 도움은 아직 연결하지 않았다. 알림 시각을 저장해도 자동 DM은 시작되지 않는다. `/setup`은 프로필 설정이며 OAuth 계정 소유 인증을 대신하지 않는다.

라이브 solved.ac 응답은 이 환경에서 아직 검증하지 않았다. 목록 응답은 배열 또는 `items` 배열, 각 항목의 문제 ID·난이도·제목·태그와 `ojLinks`의 HTTPS 링크를 읽는다. 서버에서 필드 형식이 다르면 임의의 링크를 만들지 않고 실패 안내를 한다. 운영에서 아래 진단을 실행하면 비밀 값 없이 HTTP 상태와 응답 필드 이름을 확인할 수 있다.

```sh
sudo python3 scripts/check-solved-ac.py
```

초기 설정, 일일 배정, 풀이 확인과 알림, 스킵과 재개, 커리큘럼, 막힌 문제 도움의 핵심 로직과 solved.ac OAuth 클라이언트가 있다. Discord 연결 확인용 바이너리를 제공하며 실서비스 연결은 아래 작업에 남겨 두었다.

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

## 출시 전 남은 작업

Debian 13 x86_64 홈서버의 바이너리 상시 실행은 [systemd 배포 안내](deploy/README.md)를 따른다.

- [ ] 봇 호스트와 OAuth 콜백 주소를 정한다. solved.ac Backstage에 앱을 하나 생성하고 정확히 같은 콜백 URL을 등록한다. Client ID와 Client Secret은 봇 실행 환경이나 비밀 저장소에 넣는다.
- [ ] Discord Developer Portal에 앱과 봇을 생성하고 Application ID와 Bot Token을 준비한다. 테스트 서버에 설치하고 슬래시 명령을 등록한다.
- [x] 봇 실행 진입점과 테스트 서버 `/ping` 명령을 구현한다.
- [ ] 학습용 Discord 명령, DM 및 스케줄러를 구현한다. 각 Discord 사용자의 계정 연결 명령에서 OAuth 링크를 보여주고, 콜백에서 state를 검증하고 토큰을 해당 사용자로 저장한다.
- [ ] solved.ac v4로 티어, 문제 검색과 실제 제출 링크, 사용자별 풀이 상태를 기존 Rust 기능에 연결한다. 사용자별 OAuth에서 `solves offline.access`를 요청하고 토큰 갱신과 만료를 검증한다. 자세한 호출 계약은 [solved.ac v4 연동 확인](docs/solved-ac-v4.md)에 있다.
- [ ] 봇 호스트의 Antigravity CLI(`agy`)를 연결한다. 목표 해석과 막힌 문제 도움의 구조화 응답, 후보 검증, 시간 제한, 실패 후 동작을 확인한다.
- [ ] 테스트 사용자 한 명으로 Discord 계정 연결 → `/setup` → `/today` → 저지 제출 → 완료 확인 → 미완료 DM을 실검증한다. 스킵·재개와 토큰 갱신도 검증한다.
- [ ] 나머지 두 사용자가 자기 solved.ac 계정으로 연결한다. 세 명 사용 시 봇 프로세스 최대 RSS가 100 MiB 이하인지 확인하고 `agy` 동시 실행 때도 측정한다.

자격 증명과 사용자별 실행 데이터는 저장소 밖에서 관리한다. FSD 요구사항과 승인 스펙은 `requirements/`, `specs/`에 있다.

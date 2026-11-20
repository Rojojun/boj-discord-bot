# boj-discord-bot

solved.ac 티어로 매일 연습 문제를 고르고, 목표별 학습과 풀이 확인을 제공하는 3인용 Discord 봇 프로젝트다. Rust로 구현하고 사용자별 JSON 파일을 사용한다.

## 현재 상태

초기 설정, 일일 배정, 풀이 확인과 알림, 스킵과 재개, 커리큘럼, 막힌 문제 도움의 핵심 로직과 solved.ac OAuth 클라이언트가 있다. 현재 Cargo 패키지는 라이브러리이며 Discord에서 실행할 바이너리와 실서비스 연결은 아래 작업에 남겨 두었다.

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

## 출시 전 남은 작업

- [ ] 봇 호스트와 OAuth 콜백 주소를 정한다. solved.ac Backstage에 앱을 하나 생성하고 정확히 같은 콜백 URL을 등록한다. Client ID와 Client Secret은 봇 실행 환경이나 비밀 저장소에 넣는다.
- [ ] Discord Developer Portal에 앱과 봇을 생성하고 Application ID와 Bot Token을 준비한다. 테스트 서버에 설치하고 슬래시 명령을 등록한다.
- [ ] 봇 실행 진입점과 Discord 명령, DM 및 스케줄러를 구현한다. 각 Discord 사용자의 계정 연결 명령에서 OAuth 링크를 보여주고, 콜백에서 state를 검증하고 토큰을 해당 사용자로 저장한다.
- [ ] solved.ac v4로 티어, 문제 검색과 실제 제출 링크, 사용자별 풀이 상태를 기존 Rust 기능에 연결한다. 사용자별 OAuth에서 `solves offline.access`를 요청하고 토큰 갱신과 만료를 검증한다. 자세한 호출 계약은 [solved.ac v4 연동 확인](docs/solved-ac-v4.md)에 있다.
- [ ] 봇 호스트의 Antigravity CLI(`agy`)를 연결한다. 목표 해석과 막힌 문제 도움의 구조화 응답, 후보 검증, 시간 제한, 실패 후 동작을 확인한다.
- [ ] 테스트 사용자 한 명으로 Discord 계정 연결 → `/setup` → `/today` → 저지 제출 → 완료 확인 → 미완료 DM을 실검증한다. 스킵·재개와 토큰 갱신도 검증한다.
- [ ] 나머지 두 사용자가 자기 solved.ac 계정으로 연결한다. 세 명 사용 시 봇 프로세스 최대 RSS가 100 MiB 이하인지 확인하고 `agy` 동시 실행 때도 측정한다.

자격 증명과 사용자별 실행 데이터는 저장소 밖에서 관리한다. FSD 요구사항과 승인 스펙은 `requirements/`, `specs/`에 있다.

# 홈서버 agy 연동

`/goal text:LRU와 LFU를 배우고 싶어요`는 트랙을 제안한다. 사용자가 `/curriculum track:...`으로 확인하기 전에는 목표를 바꾸지 않는다.
`/hint problem:1`은 오늘 추천 1번의 제목·태그 기반 선수 개념과 검증된 더 쉬운 연습 후보를 안내한다. 슬롯 교체는 이번 기능에 포함하지 않는다. 학습 중단 중에도 오늘 저장된 추천이 있으면 읽을 수 있다.

봇의 `bojbot`과 AI의 `bojai`를 분리한다. 봇은 Unix socket으로 학습 목표 또는 제목·태그·후보 ID만 보낸다. Discord ID, 핸들, URL, 채팅 기록과 자격 증명은 전달하지 않는다. AI 서비스는 봇 설정·데이터 디렉터리에 접근할 수 없다. CLI 호출은 shell 없이 구성하고 환경변수를 정리하며, 한 번에 한 요청만 실행한다. 호출 시간은 최대 60초, 출력은 64 KiB로 제한하고 초과 시 프로세스 그룹을 종료한다.

공식 CLI 규격의 `-p`, `--model`, `--mode plan`, `--sandbox`, `--json-schema`, `--output-format json`, `--print-timeout`을 사용한다. `SUCCESS`와 `structured_output`만 읽는다. strict 및 모든 도구 deny 규칙을 적용한다. 이 호스트의 실제 CLI 로그인, 모델 호출, sandbox·deny 작동과 메모리는 아래에서 검증해야 한다. 자동 테스트 통과만으로 운영 실행이 검증되지는 않는다.

## 1. 설치 경로와 모델 확인

홈서버에서 기존 로그인 사용자로 실행한다. 결과에 비밀 값을 포함하지 않는다.

```sh
command -v agy
agy --version
agy models
```

확인한 실행 경로와 모델 ID로 프로젝트 폴더에서 실행한다.

```sh
sudo sh scripts/install-agy-worker.sh /실제/경로/agy 실제_모델_ID
```

`/실제/경로/agy`와 모델 ID는 예시 그대로 실행하지 않는다. 스크립트는 CLI를 `/usr/local/bin/agy`로 배치하고 별도 계정·정책·서비스를 설치한다. 아직 서비스를 시작하지 않는다.

## 2. AI 전용 사용자 인증

기존 `rojojun` 사용자의 로그인은 새 서비스 사용자에게 자동 공유되지 않는다. 인증 파일을 복사하지 말고 전용 사용자로 한 번 로그인한다.

```sh
sudo -u bojai -H sh -c 'cd /var/lib/boj-ai/work && exec /usr/local/bin/agy'
```

CLI의 로그인 안내를 완료한 뒤 종료한다. 인증 과정에서 설정 파일이 바뀔 수 있으므로 관리 정책을 다시 적용한다.

```sh
sudo install -o bojai -g bojbot -m 600 deploy/agy-settings.json /var/lib/boj-ai/.gemini/antigravity-cli/settings.json
sudo -u bojai -H /usr/local/bin/agy models
```

실제 모델 ID가 `/etc/boj-ai/worker.env`의 `AGY_MODEL`과 일치하는지 확인한다.

## 3. 격리 및 실제 호출 검증

```sh
sudo -u bojai test ! -r /etc/boj-discord-bot/bot.env && echo '봇 비밀 파일 접근 차단 확인'
sudo -u bojai test ! -r /var/lib/boj-discord-bot && echo '봇 데이터 접근 차단 확인'
sudo systemctl enable --now boj-ai
sudo journalctl -u boj-ai -n 30 --no-pager
```

`isolated agy worker ready`가 나온 뒤 봇 최신 바이너리를 배포·재시작한다. 기본 소켓은 `/run/boj-ai/worker.sock`이며 바꾸려면 봇 환경변수 `AGY_SOCKET`을 사용한다. 소켓 그룹은 `bojbot`이다. 서비스가 없거나 인증·모델·sandbox·응답 검증이 실패하면 `/goal`은 직접 트랙 선택 목록, `/hint`는 태그 기반 안내로 돌아간다. 정규 봇 기능은 계속 사용할 수 있다.

Discord에서 `/goal text:LRU와 LFU를 배우고 싶어요`로 실제 모델 호출을 확인한다. 제안만 표시하며 스킵·진행은 바뀌지 않는다. `/hint problem:1`은 오늘 추천이 있는 날 검증한다. 연습 후보가 없으면 후보 부족을 표시한다. 요청 폭주 시 동시에 한 요청만 처리하고 나머지는 fallback을 반환한다.

전용 계정의 strict/deny 설정이 실제로 적용되는지도 관리자 터미널에서 확인한다. `agy --sandbox --mode plan -p`로 파일 읽기·명령·웹 도구 시도를 요청해 거부되는지 관찰한다. 프롬프트에는 실제 비밀 경로나 내용을 사용하지 않고 별도 테스트 파일을 사용한다. 설정을 완화하거나 `--dangerously-skip-permissions`로 우회하지 않는다. sandbox가 동작하지 않는 호스트에서는 AI 기능을 활성 운영하지 않는다.

## 4. 메모리와 로그

```sh
sudo systemctl show boj-discord-bot boj-ai -p MemoryCurrent -p MemoryPeak
sudo journalctl -u boj-ai -f
```

봇 자체 최대 RSS 100 MiB 목표와 CLI 동시 실행 사용량은 별도로 측정한다. AI 서비스에는 768 MiB 상한과 프로세스 수 제한을 둔다. 서비스 로그는 프롬프트·응답·CLI 인증 오류 본문을 출력하지 않는다.

참고: [공식 headless 규격](https://antigravity.google/docs/cli/headless/), [도구 권한](https://antigravity.google/docs/permissions?tab=cli).

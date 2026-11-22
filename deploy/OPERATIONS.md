# 3명 운영 확인

## 적용

홈서버에서 일반 사용자로 실행한다.

```sh
cd ~/boj-discord-bot
git pull --ff-only && sh scripts/deploy-server.sh
sudo journalctl -u boj-discord-bot -n 20 --no-pager
```

빌드 완료 후 바이너리를 교체한다. 기존 데이터, 환경변수, OAuth/agy 인증 파일은 건드리지 않는다. AI 워커가 설치되어 있으면 코드와 함께 관리 정책 `settings.json`을 다시 적용한다. CLI 사용 중 설정이 달라졌을 때 정책 검사 실패를 예방한다. 정책 파일도 설치 전 백업하며 설치 실패 시 복구한다. AI 서비스가 설치되어 실행 중이면 워커 코드와 unit을 갱신하고 재시작한다. 중지해둔 AI는 임의로 켜지 않는다. 설치 실패 시 백업 파일로 복구를 시도하고 백업 경로를 출력한다. 백업은 `/opt/boj-discord-bot/.release-backup.*`에 보관되므로 새 버전이 안정적이면 관리자가 오래된 백업을 정리한다. 바이너리 복구는 외부 API 동작이나 Discord 연결 성공을 보증하지 않는다.

Discord에서 `/version`으로 설치한 Git 커밋을 확인한다. 연결 로그 후 명령을 사용한다. 변경 없는 명령은 시작할 때 다시 등록하지 않아 재시작 대기를 줄인다.

## 각 사람의 확인

각자 자신의 Discord 계정과 solved.ac 계정으로 진행한다. 다른 사람의 토큰을 복사하지 않는다.

| 확인 | 실행 | 기대 결과 |
|---|---|---|
| 설정 | `/settings` 또는 `/setup handle:본인 count:2` | 자기 핸들·문제 수 |
| 계정 | `/connect` → 승인 → `/account` | 자기 solved.ac 계정 |
| 개인 알림 | `/settings delivery:dm` → `/notify-test` | 본인 DM 도착, 공개 채널로 우회하지 않음 |
| 공개 알림 | 원하는 사람만 `/settings delivery:channel` → `/notify-test` | 지정 채널의 본인 멘션 |
| 새 추천 | 다음 학습일 `/today` | 본인 미완료 확인 후보. 부족하면 부족 안내 |
| 풀이 확인 | 추천 문제 제출 후 `/check` | 반영 후 해당 문제만 해결 확인 |
| 목표 | `/goal text:캐시를 배우고 싶어요` → `/curriculum track:cache` | 사용자가 선택할 때 적용, 당일 목록 유지 |
| 힌트·교체 | `/hint problem:1` → 제안된 `/replace offer:...` | 본인·당일·미해결 재확인 후 그 슬롯만 교체 |
| 진도 | 같은 단계의 검증된 새 추천에서 2개 해결 후 `/check` → `/curriculum` | 다음 단계 한 번 이동, 재조회로 추가 이동 없음 |
| 정지·재개 | `/skip days:1` → 만료 후 안내 확인 → `/resume` | 쉬는 동안 자동 독촉 없음, 명시적 재개 전 정지 유지 |

쉬는 사람을 테스트하려고 임의로 재개하거나 추천을 새로 만들 필요는 없다. `/notify-test`와 `/hint`는 요청한 읽기/수신 테스트로 사용할 수 있다. 자동 완료 알림은 오늘 추천이 있고 알림 시각 이후 미완료가 남은 사람에게만 보낸다. 풀이 상태 확인 대기라면 독촉하지 않는다. DM을 막았으면 Discord 서버의 개인 메시지 허용을 확인한다. 같은 날짜에 이미 보낸 자동 알림은 방식 변경·재시작·문제 교체 후에도 반복하지 않는다.

기존 추천 파일에는 배정 시점의 미해결 증명이나 단계가 없을 수 있다. 기존 파일을 덮어쓰거나 소급 진도로 계산하지 않는다. 다음 학습일에 새로 추천받은 문제부터 적용된다. 이미 해결된 문제나 API가 null을 반환한 문제는 새 연결 추천에서 제외된다.

## 토큰 만료와 재시작

토큰은 만료 30초 전부터 요청 시 갱신한다. 실제 만료 후 `/check` 또는 다음 신규 `/today`를 실행해 성공하는지 확인한다. 운영 파일의 만료 시각이나 refresh token을 강제로 편집하지 않는다. 명확한 429 응답은 서버가 지정한 대기 시간 이후 재시도한다. 응답이 유실되면 일회성 refresh token을 재사용하지 않고 `/connect`를 요구할 수 있다.

세 사람 모두 설정한 뒤 다음을 실행한다. 사용자 3명·연결 3개·서로 다른 계정 3개인지 확인한다. 비밀 값은 출력하지 않는다.

```sh
sudo python3 scripts/audit-runtime.py
sudo systemctl restart boj-discord-bot
sudo systemctl is-active boj-discord-bot boj-ai
```

재시작 후 각자 `/account`, `/settings`, `/status`, `/curriculum`을 확인한다. 토큰 갱신·알림 일일 기록·교체 버전·커리큘럼 진행은 파일에 유지된다. 아직 누르지 않은 OAuth 링크는 재시작 후 `/connect`로 새로 받는다.

## 메모리와 장애 확인

세 명이 실제 사용하는 동안 `/goal`이나 `/hint`를 실행하고 감사 도구를 다시 실행한다. `/proc`의 봇 `VmRSS`와 `VmHWM` 목표는 100 MiB 이하이며 AI 메모리는 별도다. `MemoryCurrent/MemoryPeak`는 systemd cgroup 값으로 RSS와 다를 수 있다. 설치된 AI 서비스는 단일 호출, 768 MiB 상한으로 제한한다.

```sh
sudo python3 scripts/audit-runtime.py
sudo journalctl -u boj-discord-bot -u boj-ai --since '5 minutes ago' --no-pager
```

이 저장소의 자동 테스트는 모의 응답과 로컬 저장소로 동작한다. 세 사람의 실제 DM 수신·API 상태 반영·자연 만료 후 갱신·실제 모델의 sandbox 동작은 각 계정과 서버에서 확인해야 한다. 저장된 연결 수만으로 성공을 판정하지 않는다. 인증 파일이나 토큰 원문은 공유하지 않는다.

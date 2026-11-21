# Debian 13 x86_64 바이너리 배포

서버에서 최초 빌드 후 systemd가 실행 파일을 직접 실행한다. 실행 중에는 Rust나 Cargo가 필요 없다.
배포 대상은 현재 구현된 `/ping`이다. 홈서버의 인터넷 연결과 DNS가 필요하며 인바운드 포트는 필요 없다.

## 1. 소스 전송 및 빌드

서버에서 일반 사용자로 저장소를 받아 빌드한다. 별도 압축파일은 필요 없다.

```sh
sudo apt update
sudo apt install -y git build-essential pkg-config ca-certificates curl
git clone https://github.com/Rojojun/boj-discord-bot.git ~/boj-discord-bot
cd ~/boj-discord-bot
```

Rust가 없다면 https://rustup.rs 의 공식 설치 명령으로 설치한다.
이 프로젝트는 edition 2024와 최신 의존성을 사용하므로 stable 도구 체인을 사용한다.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/bojbot-rustup.sh
sh /tmp/bojbot-rustup.sh --profile minimal -y
sh scripts/build-release.sh
```

`-j 1`로 빌드 작업 수를 제한한다. 4 GiB 서버라도 다른 서비스 사용량에 따라 빌드 메모리가 부족할 수 있다.

## 2. 최초 설치

프로젝트 디렉터리에서 실행한다. 서비스 계정은 로그인 없이 봇만 실행한다.

```sh
id bojbot >/dev/null 2>&1 || sudo useradd --system --user-group --home-dir /var/lib/boj-discord-bot --shell /usr/sbin/nologin bojbot
sudo install -d -o root -g root -m 755 /opt/boj-discord-bot
sudo install -d -o root -g root -m 700 /etc/boj-discord-bot
sudo install -d -o bojbot -g bojbot -m 700 /var/lib/boj-discord-bot
sudo install -m 755 target/release/boj-discord-bot /opt/boj-discord-bot/boj-discord-bot
sudo install -m 644 deploy/boj-discord-bot.service /etc/systemd/system/boj-discord-bot.service
sudo test -e /etc/boj-discord-bot/bot.env || sudo install -m 600 .env.example /etc/boj-discord-bot/bot.env
sudo nano /etc/boj-discord-bot/bot.env
```

`DISCORD_BOT_TOKEN`에 재발급한 토큰을 넣는다. 예시의 서버와 채널 ID가 맞는지 확인한다.
실제 토큰은 서버 설정 파일에만 저장하고 소스나 배포 압축에 넣지 않는다.
로컬에서 실행하던 봇을 Ctrl+C로 종료한 후 서버 서비스를 시작한다.

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now boj-discord-bot
sudo systemctl status boj-discord-bot --no-pager
sudo journalctl -u boj-discord-bot -n 50 --no-pager
```

연결 완료 로그를 확인한 뒤 지정한 Discord 채널에서 `/ping`을 실행한다.
실패를 반복해 시작 제한에 걸리면 설정을 수정하고 `sudo systemctl reset-failed boj-discord-bot` 후 다시 시작한다.

## 운영 및 업데이트

```sh
sudo journalctl -u boj-discord-bot -f
sudo systemctl restart boj-discord-bot
sudo systemctl stop boj-discord-bot
```

새 소스로 빌드한 후 서비스를 중지하고 실행 파일만 교체한다.
사용자 설정은 `/etc/boj-discord-bot/bot.env`, 진행 데이터용 작업 디렉터리는 `/var/lib/boj-discord-bot`이므로 업데이트 시 보존된다.
현재 학습 로직은 아직 실행부에 연결되지 않았다.

```sh
cd ~/boj-discord-bot
git pull --ff-only
sh scripts/build-release.sh
sudo systemctl stop boj-discord-bot
sudo cp /opt/boj-discord-bot/boj-discord-bot /opt/boj-discord-bot/boj-discord-bot.previous
sudo install -m 755 target/release/boj-discord-bot /opt/boj-discord-bot/boj-discord-bot
sudo systemctl start boj-discord-bot
```

문제가 생기면 서비스를 중지하고 `.previous` 파일을 원래 실행 파일 위치로 복원한 뒤 시작한다.

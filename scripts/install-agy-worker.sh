#!/bin/sh
set -eu
[ "$(uname -s)" = Linux ] || { printf '%s\n' 'Debian 홈서버에서 실행하세요.' >&2; exit 1; }
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
if [ "$(id -u)" -ne 0 ]; then
    printf '%s\n' 'sudo sh scripts/install-agy-worker.sh /absolute/path/to/agy MODEL_ID' >&2
    exit 1
fi
agy_source=${1:?설치된 agy 실행 파일의 절대 경로가 필요합니다}
agy_model=${2:?agy models에 표시된 정확한 모델 ID가 필요합니다}
case "$agy_source" in /*) ;; *) exit 1 ;; esac
case "$agy_model" in ''|*[!A-Za-z0-9._-]*) printf '%s\n' '잘못된 모델 ID입니다.' >&2; exit 1 ;; esac
id bojbot >/dev/null
python3 - "$agy_source" <<'PY'
import sys
with open(sys.argv[1], 'rb') as binary:
    if binary.read(4) != b'\x7fELF':
        sys.exit('Linux 네이티브 agy 바이너리가 필요합니다. 래퍼 스크립트를 복사하지 않습니다.')
PY
if [ "$(readlink -f "$agy_source")" != /usr/local/bin/agy ]; then
    install -m 755 "$agy_source" /usr/local/bin/agy
fi
id bojai >/dev/null 2>&1 || useradd --system --gid bojbot --home-dir /var/lib/boj-ai --shell /usr/sbin/nologin bojai
install -d -o bojai -g bojbot -m 700 /var/lib/boj-ai /var/lib/boj-ai/work /var/lib/boj-ai/.gemini /var/lib/boj-ai/.gemini/antigravity-cli
install -d -m 700 /etc/boj-ai
install -d -m 755 /opt/boj-discord-bot
install -m 644 scripts/agy-worker.py /opt/boj-discord-bot/agy-worker.py
install -o bojai -g bojbot -m 600 deploy/agy-settings.json /var/lib/boj-ai/.gemini/antigravity-cli/settings.json
if [ ! -e /etc/boj-ai/worker.env ]; then
    (umask 077; printf '%s\n' 'AGY_BIN=/usr/local/bin/agy' "AGY_MODEL=$agy_model" > /etc/boj-ai/worker.env)
else
    printf '%s\n' '기존 /etc/boj-ai/worker.env를 보존했습니다. 모델 변경은 이 파일을 편집하세요.'
fi
install -m 644 deploy/boj-ai.service /etc/systemd/system/boj-ai.service
systemctl daemon-reload
printf '%s\n' 'AI 서비스 파일 준비 완료. 아직 서비스를 시작하지 않았습니다.' '다음 단계는 deploy/AGY.md의 전용 계정 인증과 실행 검증입니다.'

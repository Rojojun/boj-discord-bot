#!/bin/sh
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
[ "$(id -u)" -eq 0 ] && [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || { printf '%s\n' 'Debian x86_64에서 sudo로 실행하세요.' >&2; exit 1; }
python3 - <<'PY'
from pathlib import Path
if Path('target/release/boj-discord-bot').read_bytes()[:4] != b'\x7fELF':
    raise SystemExit('Linux 릴리스 바이너리가 필요합니다.')
PY
systemctl cat boj-discord-bot >/dev/null
install -d -m 755 /opt/boj-discord-bot
release_backup=$(mktemp -d /opt/boj-discord-bot/.release-backup.XXXXXX)
chmod 700 "$release_backup"
release_had_binary=0
release_has_ai=0
release_ai_running=0
if [ -f /opt/boj-discord-bot/boj-discord-bot ]; then
    cp -p /opt/boj-discord-bot/boj-discord-bot "$release_backup/bot"
    release_had_binary=1
fi
if [ -f /etc/systemd/system/boj-ai.service ]; then
    release_has_ai=1
    cp -p /etc/systemd/system/boj-ai.service "$release_backup/ai.service"
    if [ -f /var/lib/boj-ai/.gemini/antigravity-cli/settings.json ]; then
        cp -p /var/lib/boj-ai/.gemini/antigravity-cli/settings.json "$release_backup/ai-policy.json"
    fi
    if [ -f /opt/boj-discord-bot/agy-worker.py ]; then cp -p /opt/boj-discord-bot/agy-worker.py "$release_backup/worker.py"; fi
    # Auto-restarting/failed workers also need recovery. Preserve an explicitly
    # stopped (inactive) worker instead of enabling AI without the operator.
    case "$(systemctl show boj-ai --property=ActiveState --value)" in
        active|activating|failed) release_ai_running=1 ;;
    esac
fi
rollback() {
    trap - EXIT HUP INT TERM
    printf '%s\n' '설치 실패. 실패 직후 서비스 로그:' >&2
    journalctl -u boj-discord-bot -u boj-ai -n 30 --no-pager >&2 || true
    printf '%s\n' '이전 실행 파일로 복구합니다.' >&2
    systemctl stop boj-discord-bot || true
    if [ "$release_had_binary" -eq 1 ]; then
        install -m 755 "$release_backup/bot" /opt/boj-discord-bot/boj-discord-bot
    fi
    if [ "$release_has_ai" -eq 1 ]; then
        cp -p "$release_backup/ai.service" /etc/systemd/system/boj-ai.service
        if [ -f "$release_backup/ai-policy.json" ]; then
            cp -p "$release_backup/ai-policy.json" /var/lib/boj-ai/.gemini/antigravity-cli/settings.json
        fi
        if [ -f "$release_backup/worker.py" ]; then cp -p "$release_backup/worker.py" /opt/boj-discord-bot/agy-worker.py; fi
        systemctl daemon-reload || true
        if [ "$release_ai_running" -eq 1 ]; then systemctl restart boj-ai || true; fi
    fi
    systemctl start boj-discord-bot || true
    printf '복구용 백업: %s\n' "$release_backup" >&2
    exit 1
}
trap rollback EXIT HUP INT TERM
# Stage the binary first, then use rename on the same filesystem.
install -m 755 target/release/boj-discord-bot /opt/boj-discord-bot/boj-discord-bot.new
systemctl stop boj-discord-bot
mv /opt/boj-discord-bot/boj-discord-bot.new /opt/boj-discord-bot/boj-discord-bot
if [ "$release_has_ai" -eq 1 ]; then
    # agy can update settings during use. Reapply the worker's managed policy
    # with its code; authentication files and worker.env remain untouched.
    systemctl stop boj-ai
    install -o bojai -g bojbot -m 600 deploy/agy-settings.json /var/lib/boj-ai/.gemini/antigravity-cli/settings.json
    install -m 644 scripts/agy-worker.py /opt/boj-discord-bot/agy-worker.py
    install -m 644 deploy/boj-ai.service /etc/systemd/system/boj-ai.service
    systemctl daemon-reload
    if [ "$release_ai_running" -eq 1 ]; then systemctl restart boj-ai; fi
fi
systemctl reset-failed boj-discord-bot
systemctl start boj-discord-bot
sleep 3
systemctl is-active --quiet boj-discord-bot
if [ "$release_ai_running" -eq 1 ]; then systemctl is-active --quiet boj-ai; fi
trap - EXIT HUP INT TERM
printf '설치 완료. 이전 파일 백업: %s\n' "$release_backup"
printf '%s\n' 'Discord /version으로 새 버전을 확인하세요. 계정 연결·설정·학습 중단 상태는 보존했습니다.'
systemctl status boj-discord-bot --no-pager -l

#!/usr/bin/env python3
"""Read-only local audit. Never print credential files, tokens, or exception text."""
import argparse
import json
from pathlib import Path
import subprocess
import time


def audit(data, expected=3):
    users = sorted(p for p in (data / 'users').glob('*') if p.is_dir() and p.name.isdigit())
    configured = 0
    accounts = []
    for directory in users:
        try:
            settings = json.loads((directory / 'settings.json').read_text())
        except FileNotFoundError:
            continue
        except (OSError, ValueError):
            print(f'사용자 {directory.name}: 설정 읽기 실패')
            continue
        configured += 1
        print(f'사용자 {directory.name}: 알림={settings.get("delivery", "channel")} 시각={settings.get("reminder_time", "21:00")}')
        try:
            session = json.loads((directory / 'solved-ac-session.json').read_text())
            account = session['account']['id']
            tokens = session['tokens']
            accounts.append(account)
            solves = 'solves' in tokens.get('scope', '').split()
            refresh = bool(tokens.get('refresh_token'))
            remaining = max(0, int(session['expires_at']) - int(time.time()))
            print(f'  OAuth 저장됨 · 풀이 권한={solves} · 갱신 수단={refresh} · 토큰 잔여={remaining}초')
        except FileNotFoundError:
            print('  OAuth 미연결')
        except (OSError, ValueError, KeyError, TypeError):
            print('  OAuth 저장 형식 확인 필요 (비밀 값은 출력하지 않음)')
    distinct = len(set(accounts))
    print(f'설정 사용자 {configured}/{expected} · 연결 {len(accounts)}/{expected} · 서로 다른 연결 계정 {distinct}')
    print('저장 상태 검사이며 실제 API 호출·토큰 갱신·DM 도착을 검증하지 않습니다.')
    return configured == expected and len(accounts) == expected and distinct == expected


def services():
    for service in ('boj-discord-bot', 'boj-ai'):
        try:
            result = subprocess.run(['systemctl', 'show', service, '--property=ActiveState,SubState,MemoryCurrent,MemoryPeak,MainPID'], capture_output=True, text=True, timeout=5)
            allowed = {'ActiveState', 'SubState', 'MemoryCurrent', 'MemoryPeak', 'MainPID'}
            rows = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line and line.split('=', 1)[0] in allowed)
            print(service + ': ' + ' '.join(f'{key}={value}' for key, value in rows.items()))
            pid = int(rows.get('MainPID', '0'))
            if pid > 0:
                for line in Path(f'/proc/{pid}/status').read_text().splitlines():
                    if line.startswith(('VmRSS:', 'VmHWM:')):
                        print('  ' + line)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            print(service + ': 서비스/메모리 조회 불가')
    print('봇 RSS 목표 100 MiB. 실제 3명 사용과 AI 호출 중에도 측정하세요.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, default=Path('/var/lib/boj-discord-bot/data'))
    parser.add_argument('--expected-users', type=int, default=3)
    args = parser.parse_args()
    audit(args.data, args.expected_users)
    services()

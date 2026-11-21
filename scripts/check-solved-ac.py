"""Inspect public response fields without printing app credentials or tokens."""
import json
from pathlib import Path
import shlex
import sys
import urllib.error
import urllib.request

try:
    config = Path(sys.argv[1] if len(sys.argv) > 1 else "/etc/boj-discord-bot/bot.env")
    values = {}
    for line in config.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = " ".join(shlex.split(value, comments=True))
    headers = {"x-client-id": values["SOLVED_AC_CLIENT_ID"], "x-client-secret": values["SOLVED_AC_CLIENT_SECRET"], "User-Agent": "boj-discord-bot diagnostic"}
    if not all(headers.values()):
        raise ValueError()
except (OSError, ValueError, KeyError):
    sys.exit("서버 설정 파일 또는 solved.ac 앱 인증값을 확인하세요. 비밀 값은 출력하지 않습니다.")

try:
    request = urllib.request.Request("https://solved.ac/api/v4/problems?page=1", headers=headers)
    with urllib.request.urlopen(request, timeout=20) as response:
        print("HTTP:", response.status)
        body = json.load(response)
    print("최상위 형식:", type(body).__name__)
    if isinstance(body, dict):
        print("최상위 필드:", ", ".join(body))
    items = body if isinstance(body, list) else body.get("items") if isinstance(body, dict) else None
    if isinstance(items, list):
        print("문제 개수:", len(items))
        if items and isinstance(items[0], dict):
            first = items[0]
            print("첫 문제 필드:", ", ".join(first))
            for field in ("problemId", "id", "level", "tier", "title", "titles", "tags", "ojLinks"):
                value = first.get(field)
                print(field + " 형식:", type(value).__name__)
                if isinstance(value, list) and value and isinstance(value[0], dict):
                    print(field + " 첫 항목 필드:", ", ".join(value[0]))
except urllib.error.HTTPError as error:
    print("HTTP:", error.code)
    if error.code == 429:
        print("Retry-After:", error.headers.get("Retry-After", "미지정"))
    sys.exit(1)
except Exception:
    sys.exit("네트워크 연결 또는 JSON 응답 해석에 실패했습니다.")

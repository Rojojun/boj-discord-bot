#!/bin/sh
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    printf '%s\n' 'Debian x86_64 서버에서 실행하세요. macOS 바이너리는 배포할 수 없습니다.' >&2
    exit 1
fi
if command -v cargo >/dev/null 2>&1; then
    exec cargo build --release --locked -j 1
fi
exec "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" build --release --locked -j 1

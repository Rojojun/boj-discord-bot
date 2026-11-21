#!/bin/sh
set -eu

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

if command -v cargo >/dev/null 2>&1; then
    exec cargo run --locked "$@"
fi

cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin/cargo"
if [ -x "$cargo_bin" ]; then
    exec "$cargo_bin" run --locked "$@"
fi

printf '%s\n' 'Cargo를 찾을 수 없습니다. https://rustup.rs 에서 Rust를 설치하세요.' >&2
exit 127

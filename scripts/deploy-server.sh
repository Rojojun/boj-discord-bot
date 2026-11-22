#!/bin/sh
# Run as the normal repository owner; sudo is used only for installation.
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
sh scripts/build-release.sh
sudo sh scripts/install-release.sh
sudo python3 scripts/audit-runtime.py

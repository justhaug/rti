#!/usr/bin/env bash
# Seeds the project root on the persistent volume, then runs `rti <args>`.
set -euo pipefail
ROOT="${RTI_ROOT:-/data/project}"
mkdir -p "$ROOT"
cd "$ROOT"
[ -d prompts ] || cp -r /opt/rti/prompts .
[ -f AGENTS.md ] || cp /opt/rti/AGENTS.md .
[ -d tracks ] || cp -r /opt/rti/tracks .
if [ ! -d .git ]; then
  git init -q . && git -c user.name=RTI -c user.email=rti@localhost add -A && git -c user.name=RTI -c user.email=rti@localhost commit -qm "init project root" || true
fi
[ -f rti.toml ] || rti init
if [ "${1:-serve}" = "serve" ]; then
  shift || true
  exec rti serve --port "${PORT:-8787}" "$@"
fi
exec rti "$@"

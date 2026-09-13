#!/usr/bin/env bash
# Local TM2020 oracle setup on Ubuntu (Steam + Proton GE + Openplanet).
# Idempotent; re-run after each manual step. Steps needing root or a GUI
# are printed, not executed.
set -euo pipefail

APPID=2225070                       # Trackmania (2020) on Steam
STEAM_ROOT="${STEAM_ROOT:-$HOME/.steam/root}"
SETUP_DIR="${SETUP_DIR:-$HOME/Downloads/rti-setup}"
GE_TAG="${GE_TAG:-}"                # empty = latest
mkdir -p "$SETUP_DIR"

say() { printf '\n== %s\n' "$*"; }
need() { printf '   -> run manually: %s\n' "$*"; }

say "1. packages"
missing=()
for p in steam-installer protontricks vulkan-tools; do
  dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
done
if ((${#missing[@]})); then need "sudo apt install -y ${missing[*]}"; else echo "   ok"; fi

say "2. Steam client"
if [ -d "$STEAM_ROOT/ubuntu12_64" ]; then echo "   ok ($STEAM_ROOT)"; else need "steam   # first launch bootstraps the client; then log in"; fi

say "3. GE-Proton"
COMPAT="$STEAM_ROOT/compatibilitytools.d"
mkdir -p "$COMPAT"
if ls -d "$COMPAT"/GE-Proton* >/dev/null 2>&1; then
  echo "   ok: $(ls -d "$COMPAT"/GE-Proton* | xargs -n1 basename | tr '\n' ' ')"
else
  api="https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases/${GE_TAG:+tags/}${GE_TAG:-latest}"
  url=$(curl -s "$api" | python3 -c "import json,sys; d=json.load(sys.stdin); print([a['browser_download_url'] for a in d['assets'] if a['name'].endswith('x86_64.tar.gz')][0])")
  echo "   downloading $url"
  curl -L -o "$SETUP_DIR/ge.tar.gz" "$url"
  curl -sL -o "$SETUP_DIR/ge.sha" "${url%.tar.gz}.sha512sum"
  (cd "$SETUP_DIR" && sed 's/ .*/  ge.tar.gz/' ge.sha | sha512sum -c -)
  tar xzf "$SETUP_DIR/ge.tar.gz" -C "$COMPAT"
  rm -f "$SETUP_DIR/ge.tar.gz" "$SETUP_DIR/ge.sha"
  echo "   installed; restart Steam so it appears under Compatibility"
fi

say "4. Trackmania"
LIB=$(python3 - "$STEAM_ROOT/steamapps/libraryfolders.vdf" "$APPID" <<'PY' 2>/dev/null || true
import re,sys
try: t=open(sys.argv[1]).read()
except FileNotFoundError: sys.exit(0)
paths=re.findall(r'"path"\s+"([^"]+)"',t)
blocks=t.split('"path"')
for i,b in enumerate(blocks[1:]):
    if f'"{sys.argv[2]}"' in b: print(paths[i]); break
PY
)
if [ -n "${LIB:-}" ] && [ -d "$LIB/steamapps/common/Trackmania" ]; then
  echo "   ok: $LIB/steamapps/common/Trackmania"
  GAME="$LIB/steamapps/common/Trackmania"; PFX="$LIB/steamapps/compatdata/$APPID/pfx"
else
  need "steam steam://install/$APPID   # or search 'Trackmania' in the Steam client (free)"
  need "then: right-click Trackmania -> Properties -> Compatibility -> force GE-Proton"
  need "launch it once so Ubisoft Connect installs into the prefix, log in, close, launch again"
  GAME=""; PFX=""
fi

say "5. Openplanet installer"
OP=$(ls "$SETUP_DIR"/OpenplanetNext_*.exe 2>/dev/null | tail -1 || true)
if [ -z "$OP" ]; then
  echo "   downloading"
  curl -sL -A "Mozilla/5.0" -e "https://openplanet.dev/download" -o "$SETUP_DIR/OpenplanetNext.exe" "https://openplanet.dev/download/get?id=330"
  OP="$SETUP_DIR/OpenplanetNext.exe"
fi
echo "   $OP"
if [ -n "$PFX" ]; then
  if [ -d "$PFX/drive_c/users/steamuser/OpenplanetNext" ]; then
    echo "   Openplanet already installed in the prefix"
  else
    need "protontricks-launch --appid $APPID '$OP'   # installer: point it at $GAME"
  fi
fi

say "6. RTI bridge plugin"
if [ -n "$PFX" ]; then
  PLUG="$PFX/drive_c/users/steamuser/OpenplanetNext/Plugins"
  echo "   plugins dir: $PLUG"
  echo "   protocol: docs/oracle.md ; reference server: crates/rti-oracle/tests/oracle.rs"
fi
say "7. RTI config"
echo "   rti.toml: [oracle] kind = \"tm2020\", tm2020_port = 27015 ; test with: rti oracle"

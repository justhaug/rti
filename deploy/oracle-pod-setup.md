# Oracle pod: milestone zero

Goal: a Runpod GPU desktop pod that runs TM2020 under Proton with Openplanet, answers RTI's
bridge protocol on port 27015, and shuts itself down when idle. Prove this before designing
anything else around real-game verification or rendering.

## Checklist

1. **Create the pod** — Runpod → Pods → the official *GPU desktop* template (Ubuntu, GPU-backed
   graphical session in the browser). Any 16 GB+ GPU is fine; pick a cheap Community Cloud one
   for setup. Add a persistent volume (≥ 60 GB; TM2020 + Steam + Proton is ~40 GB). Expose TCP
   ports **27015** (bridge) and **27016** (watchdog) in the pod's port settings.
2. **Open the desktop** in the browser and install Steam (`apt install steam` or the .deb).
3. **Proton** — in Steam: Settings → Compatibility → enable Steam Play for all titles, pick
   Proton Experimental (or the version TMRL / the community currently recommends).
4. **Ubisoft Connect + TM2020** — install Trackmania from Steam (it installs Ubisoft Connect
   inside the same prefix). Log in, let it update, launch once, set a fixed resolution
   (1920×1080 windowed) and lowest graphics that still look fine for rendering.
5. **Openplanet** — install the Linux/Proton build into the same prefix
   (https://openplanet.dev). Confirm the overlay shows in game.
6. **Bridge plugin** — install the RTI bridge plugin (yours; contract in `docs/oracle.md`)
   into `OpenplanetNext/Plugins/`. It must listen on 0.0.0.0:27015 and answer `hello`, `ping`,
   `load_map`, `run`.
7. **Watchdog** — copy `rti-oracle-watchdog` (built by `deploy/Dockerfile`) to the pod,
   `export RUNPOD_API_KEY=...` (`RUNPOD_POD_ID` is set by Runpod), run it under systemd
   (`deploy/oracle-watchdog.service`) or supervisord. `curl localhost:27016/health` → `ok`.
8. **Snapshot** — with everything working, save the pod as a template/volume so re-creation
   is one click.
9. **Drive one track** — from the coordinator:
   ```
   rti cloud oracle start           # resumes the pod, waits for the bridge
   rti oracle                       # "oracle tm2020 available (10 ms/tick)"
   rti verify <trajectory-hash>     # replays inputs in the real game, records divergence
   rti cloud oracle stop
   ```
   The track needs `tm_map_uid` and `tm_frame` (see docs/oracle.md).

## Configuration on the coordinator

```toml
[oracle]
kind = "tm2020"
tm2020_host = "<pod public ip or proxy host>"
tm2020_port = 31234          # the public port Runpod mapped to 27015

[cloud]
provider_oracle = "runpod"
runpod_pod_id = "abcd1234"
oracle_idle_shutdown_minutes = 10
oracle_daily_hours = 2.0
```

Runpod's public port mapping can change between starts; `rti cloud oracle status` prints the
current mapping (`public_endpoint(27015)`) so you can update `tm2020_port`, or put the pod behind
a fixed proxy.

## What we know and don't

TM2020 + Proton + Openplanet works on Linux (TMRL documents it). Running it inside a Runpod
desktop pod specifically has not been documented anywhere we found; if the graphical session or
Ubisoft Connect misbehaves there, fall back to an AWS g4dn.xlarge with an NVIDIA driver and a
remote desktop — the RTI side is identical, only `provider_oracle` and the start/stop calls change.

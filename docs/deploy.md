# Deployment: cheap, boring, budget-capped

RTI is designed to run unattended on a small always-on machine, borrowing expensive hardware
only when it has something worth verifying, rendering or training.

```
                      RTI
                       │
          ┌────────────┴─────────────┐
          │                          │
     Fly Machine                Runpod GPU Pod
     always alive               normally STOPPED
          │                          │
     rti serve                  Ubuntu desktop
     researcher (OpenRouter)    Steam + Proton
     DuckDB + git               Ubisoft + TM2020
     scheduler + UI             Openplanet + bridge
     CPU simulator              watchdog :27016
          │                          │
          └──────── R2 / S3 ─────────┘
                    artifacts
          disposable Fly workers for heavy CPU experiments
```

| piece | where | cost (2026, rough) |
|---|---|---|
| coordinator (`rti serve`) | Fly shared-cpu-2x / 2 GB + 20 GB volume | $10–25 / month |
| researcher / coder / critic | OpenRouter, GLM 5.3 Flash | cents per day at default budgets |
| TM2020 oracle + video render | Runpod GPU desktop pod, stopped when idle | ~$0.20 / GB / month for the stopped volume; $0.3–0.7 / h while running |
| heavy CPU experiments | Fly performance-4x machines, auto-destroy | ~$0.18 / h while running |
| artifacts | Cloudflare R2 (S3-compatible) | ~$0.015 / GB / month, no egress fees |

Nothing here needs Kubernetes, a queue, or a second database. DuckDB on the volume is the
archive; `cas/` is mirrored to R2; git holds code lineage.

## 1. Coordinator on Fly

```bash
fly launch --no-deploy --copy-config --name rti      # uses deploy/fly.toml
fly volumes create rti_data --size 20 --region iad
fly secrets set OPENROUTER_API_KEY=sk-or-... RUNPOD_API_KEY=... FLY_API_TOKEN=$(fly tokens create deploy -x 999999h)
fly deploy --dockerfile deploy/Dockerfile
fly proxy 8787:8787          # then open http://127.0.0.1:8787/
```

The image (`deploy/Dockerfile`) contains `rti`, `rti-oracle-watchdog`, ffmpeg and rclone. On
first boot `deploy/entrypoint.sh` seeds `/data/project` (prompts, tracks, `rti.toml`, a git
repo) on the volume and runs `rti serve`. Edit `/data/project/rti.toml` through `fly ssh console`.

Fly shared CPUs have a small baseline quota; the coordinator is fine for the researcher, the
archive and small searches, but sustained multi-core simulation belongs on workers.

### Alternative: any Linux VM

Build the binary, copy `deploy/rti.service` to `/etc/systemd/system/`, put secrets in
`/etc/rti/env`, `systemctl enable --now rti`. Same layout, same commands.

## 2. Oracle pod on Runpod

Follow `deploy/oracle-pod-setup.md` once, then set in `rti.toml`:

```toml
[oracle]
kind = "tm2020"
tm2020_host = "<pod ip>"
tm2020_port = 31234

[cloud]
provider_oracle = "runpod"
runpod_pod_id = "<id>"
oracle_idle_shutdown_minutes = 10
oracle_daily_hours = 2.0
oracle_boot_timeout_secs = 600
```

Lifecycle (`rti-infra::OracleLifecycle`):

```
verification/render needed
  → budget check (daily oracle hours)
  → Runpod podResume
  → poll bridge `ping` until available (≤ oracle_boot_timeout_secs)
  → run the jobs, sending POST :27016/heartbeat as it goes
  → idle ≥ oracle_idle_shutdown_minutes → podStop
```

Two shutdown paths exist on purpose: RTI stops the pod when idle, and the pod-side
`rti-oracle-watchdog` stops it independently when heartbeats stop, so a crashed or wedged
coordinator cannot leave a GPU running for weeks.

```bash
rti cloud oracle status     # pod state, public port mapping, hours used today
rti cloud oracle start
rti cloud oracle stop
```

## 3. Workers on Fly (optional, later)

```bash
rti cloud worker spawn --hours 0.5 -- experiment '{"kind":"search","track":"hairpin","method":"cem","budget_ticks":200000000}'
```

creates a `performance-4x` machine from `fly_worker_image`, which runs `rti <args>` against a
scratch project and then `rti cloud sync --push`; the coordinator later runs
`rti cloud sync --pull`. `max_parallel_workers` and the compute budget gate every spawn.

## 4. Artifacts on R2

Configure rclone with an `r2` remote (`rclone config`, S3 provider Cloudflare), then:

```toml
[cloud]
s3_bucket = "rti"
s3_prefix = "prod"
sync_command = "rclone sync {src} {dst}"
```

`rti cloud sync --push` snapshots `rti.duckdb` to `snapshots/` (run it with the server stopped,
DuckDB must not be open) and mirrors `cas/` and `snapshots/`; `--pull` restores them on a fresh
machine.

## 5. Budgets

All in `[cloud]`, persisted in `data/cloud_state.json`, enforced independently of the LLM:

```toml
openrouter_daily_usd = 2.0
openrouter_monthly_usd = 30.0
oracle_daily_hours = 2.0
oracle_idle_shutdown_minutes = 10
compute_daily_usd = 5.0
compute_monthly_usd = 75.0
max_oracle_instances = 1
max_parallel_workers = 4
require_approval_above_usd = 10.0
```

Anything above `require_approval_above_usd` is refused with a `NeedsApproval` message instead of
being spent; the operator/UI is where a human says yes.

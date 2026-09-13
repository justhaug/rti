# Media: RTI's public research journal

Every interesting, verified discovery already contains everything needed to make a video.
`rti-media` turns it into a 9:16 Short and puts it in front of you for review.

```
verified discovery
   ↓ detect.rs     I = w1·WR improvement + w2·technique novelty + w3·leaderboard significance + w4·visual weirdness
   ↓ compare.rs    align new vs old run by track progress: per-section deltas, exit speeds, walls, biggest gain
   ↓ job.rs        declarative MediaJob {track, new/old trajectory, camera, comparison mode, overlay, size, fps}
   ↓ render.rs     frames: title card → PART 1 new run → PART 2 old run + new ghost → WHY IT WORKS (slow-mo) → THE DIFFERENCE
   ↓ compose.rs    FFmpeg (libx264, yuv420p, faststart) → data/media/<id>.mp4
   ↓ narration.rs  title / description / explanation from the comparison and the archive lineage (LLM with template fallback)
   ↓ archive       `media` row, status = review
   ↓ UI            "POTENTIAL DISCOVERY" card with the video and Publish / Upload private / Approve / Ignore
   ↓ publish.rs    YouTube Data API v3 resumable upload (private first), then set public
```

## Triggers

`scan` looks at every track's best finished run (oracle-verified by default; `--all` includes sim
bests), compares it with the previous best and with the human reference from the imported map
(TMX WR, else author time), and scores it. Anything at or above `[media] weights.threshold`
(default 0.6 on a 0–2 scale) is rendered. A 4 ms gain on a mature track scores low; a large gain
that uses a wall differently, or beats the human reference, scores high. Each best is rendered
at most once.

The scan runs automatically after every research cycle (`rti research --with-tasks`, and the
server loop) and on demand: `rti media scan`, `rti media render <track>`, or the buttons in the
media tab.

## Rendering backends

* **Simulator renderer** (built in): top-down chase camera over the pre-rasterised track,
  car + ghost with trails, HUD (time, speed, live delta vs ghost, wall count), bitmap font, no
  external assets. Works everywhere, from archived telemetry.
* **Game renderer** (planned): the same `MediaJob` handed to the TM2020 oracle pod
  (`docs/oracle-pod.md`) which loads the map and replays, records with NVENC and returns the
  file. The composition, narration and publishing steps do not change.

FFmpeg is required for composition: install it or set `RTI_FFMPEG=/path/to/ffmpeg`.
Video files live in `data/media/`, and are also stored in the content-addressed store.

## Review and publishing

Nothing is published without a decision. States: `review → approved → uploaded (private) →
published`, or `rejected`. `rti media publish <id>` (or the Publish button) uploads if needed
and flips the video to public. Configure YouTube with an OAuth client and a refresh token in
`YOUTUBE_CLIENT_ID`, `YOUTUBE_CLIENT_SECRET`, `YOUTUBE_REFRESH_TOKEN` (`[media] youtube` lets you
rename the variables). New API projects can only upload private videos until Google's compliance
audit passes; the private-first flow is designed for exactly that.

`[media] auto_publish = true` with `auto_publish_threshold` enables unattended publishing for
verified results above the threshold once you trust the taste of the detector.

## Configuration (`rti.toml`)

```toml
[media]
enabled = true
require_verified = true
width = 1080
height = 1920
fps = 30
crf = 23
auto_publish = false
auto_publish_threshold = 1.5

[media.weights]
wr_improvement = 1.0
technique_novelty = 1.0
leaderboard_significance = 1.0
visual_weirdness = 0.7
threshold = 0.6
```

## Beyond WR shorts

The same pipeline can produce human-WR-vs-RTI overlays (import the map, ingest the replay as a
trajectory, compare), weekly compilations, and "the three ticks that matter" clips: every video
is a `MediaJob` over archived trajectories, so new formats are new job kinds, not new
infrastructure.

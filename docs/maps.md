# Map ingestion: Trackmania Exchange → simulator

`rti-maps` turns real TM2020 maps into simulator tracks so that RTI can be pointed at any map:

```
TMX id / URL / .Map.Gbx
   ↓ tmx.rs          TmxClient: search, download (/mapgbx/{id}), replay leaderboard
   ↓ gbx.rs          GBX container (header chunks, LZO body), CGameCtnChallenge decoding
   ↓                 block placements (name, dir, coord, flags, waypoint tags, free-block transforms), items (best effort)
   ↓ catalog.rs      block name → geometry template (straight / curve / chicane / marker), surface, width
   ↓ compile.rs      chain pieces start → finish through cell connectors, bridge small gaps → rti_core::Track
   ↓                 archive: `maps` row, .Map.Gbx + parsed JSON in the CAS, tracks/<name>.toml
```

Use it from the CLI (`rti map import 356566`, `rti map search --name kacky`), the research loop
(`{"kind":"import_map","source":"356566"}`) or the operator chat in the UI ("import TMX map 356566
and search it with beam").

## What is decoded

* Header: map uid, name, author, medal times, checkpoint/lap counts, the XML header.
* Body chunk `0x0304301F`: every block with name, direction (0-3), grid coordinate
  (32 m × 8 m × 32 m cells), flags (ground, free, variant), skin file, waypoint tag
  (`Spawn`, `Checkpoint`, `LinkedCheckpoint`, `Goal`) and order.
* Chunk `0x0304305F`: absolute position + pitch/yaw/roll for free blocks.
* Chunk `0x03043040`: anchored items (model name, position) — best effort; item chunk layouts
  vary by version and decoding stops at the first unknown layout (partial list kept).
* Unknown non-skippable chunks (e.g. embedded MediaTracker clips) trigger a scan-forward to the
  next skippable chunk, so later data is still decoded.

Everything is best effort and reported: `ParsedMap.warnings`, `CompileReport.unrecognized`.

## Conventions that were verified on real maps

Checked by hand on several TMX maps and on the weekly-short map from a replay
(`cargo run --release -p rti-maps --example chaindebug -- map.Map.Gbx` prints pieces and break points):

* **Placement**: a block's coordinate is the minimum corner of the *rotated* footprint's bounding
  box (cells are 32 m × 8 m × 32 m). Direction `d` rotates the piece `d × 90°` counter-clockwise in
  the (x, z) plane.
* **CurveN**: N×N footprint; the road enters through the top edge of the last column and leaves
  through the left edge of the first row; the arc is centred on the footprint's top-left corner
  (radius N·32 − 16). Holds for Curve1, Curve2, Curve3, banked/tilted variants.
* **ChicaneXN Right/Left**: N cells long; "Right" shifts the exit one cell toward −u, "Left" toward +u.
* **Straights, markers, slopes, tilts, transitions, specials**: 1 cell (or `X2`, `2x1`... long),
  ports on the two ends.
* **Open surfaces** (`*Base`, `*Base2x2`, platform diagonals, junctions): ports on every edge; the
  chainer may cross them straight or with a quarter turn.
* **Free blocks**: absolute position from chunk `0x0304305F`, yaw about the vertical axis (sign
  still tried both ways).
* Heights are used only as a matching penalty (ports more than 24 m apart in y never connect).

The chainer is a Dijkstra search from every start block and every dead end; the track is the
chain with the most road, with bonuses for reaching a finish and starting at a start block.

## Catalog coverage

Grammar-based: `catalog::tokens` splits a block name into CamelCase tokens; family tokens give the
surface, shape tokens give geometry, and a list of ignore tokens (Wall, Pillar, Loop, Structure,
Land, Tree, Cliff, ...) marks scenery. `catalog.toml` in the project root can add regex overrides
and extra ignore tokens. Diagonal *road* pieces, loops, wall rides and branches with real
geometry are not modelled.

Coverage on 142 awarded TMX maps (`example census`): about a third of all blocks are drivable
pieces, 29 % of those end up on the chosen chain, and 54 maps chain start-to-finish. Maps built
from plain road pieces compile completely (e.g. "How to map": 71/71 pieces, 3.8 km,
start-to-finish); platform fields and big height changes are where chains still break.

## Catalog coverage

The embedded catalog (`catalog::DEFAULT_CATALOG`) knows the road families
`RoadTech/Dirt/Bump/Ice`, `Open*Road`, `SnowRoad`, `RallyRoad`, `DesertRoad`, `Platform*` for the shapes
Straight, CurveN, ChicaneX2/X3, Slope/Tilt variants that behave like straights, and the race
markers, plus `Gate*` markers. Decoration, land, pillars, walls and most transition pieces are
unrecognised; the compiler bridges up to three unrecognised cells between two recognised pieces
with a straight segment. Coverage on random TMX maps today is partial: expect a drivable
*prefix* of the map (start → wherever the road could no longer be followed) rather than the
whole map, unless the map is built from plain road pieces.

Extending coverage is data work, and it is exactly the kind of task the research loop can hand
to the coding agent: add templates to `catalog.toml` in the project root (same format as
`DEFAULT_CATALOG`; entries there take precedence), or add shapes to `compile::local_geometry`.
The `chain` example prints, for a map, which blocks were recognised and how far the chain got:

```
cargo run -p rti-maps --example chain -- some.Map.Gbx
cargo run -p rti-maps --example dump  -- some.Map.Gbx     # header, chunks, block name histogram
```

## Planned (not built)

* Full vanilla asset catalog with collision meshes / surfaces extracted from the game files, and
  embedded custom items — needed for a 3D simulator; the current simulator is planar.
* Nadeo record ghosts (TMX replays and local autosaves are already ingested, see `docs/oracle.md`).
* Bulk ingestion (`search TMX → download N maps → compile → classify`) as a scheduled task.


## The 3D world and the accuracy benchmark

`compile_track3_full` returns, besides the track polyline, a **layered heightfield**
(`rti_sim::World`): every recognised block is rasterised at 1 m into (height, surface, drivable,
road-or-open) samples, several layers per cell for overpasses. Block heights come from
`shape3d.rs` (slope, slope start/end, tilt banking profiles) with the per-port height offsets
**mined from the corpus** (`crates/rti-maps/data/heights.toml`, `example heightmine`): for every
pair of adjacent pieces in 144 maps we record the neighbour's base height relative to ours, and
take the median per block name and port. That is how we know a `RoadTechSlopeStraight` spans
−8 to +8 m around its block height and a `Platform*` block is a solid whose drivable top is 8 m up.

On top of the block geometry, the chained route is rasterised as a **corridor** so that blocks we
do not model (loops, wall rides, diagonals) leave no hole in the surface. Real piece geometry
overlays it.

The car then drives on that surface: gravity along the slope, banking through the surface normal,
take-off and landing, falling off open surfaces, walls on road pieces whose outward normal is
derived from the drivable mask. Race triggers are the marker pieces of the chained route, not
centerline distances.

### Measured accuracy (`example replaybench`, 126 TMX replays)

| metric | value |
|---|---|
| routes whose compiled length is plausible for the real time | 60 of 126 |
| of those, runs the sim carries to the finish | 8 |
| within 25 % of the real time | 3 |
| median absolute time error of finished runs | 5.2 s |

The binding constraint is geometry, not the physics constants. Almost every failure is the car
leaving the drivable surface in the first 10–15 % of the route, usually with zero wall contacts:
it is on an open platform surface or a piece whose modelled shape differs from the real one, and
once the line diverges from the human's, an open-loop replay of their inputs cannot recover.
Widening the corridor does not help (tested at +8 m and +20 m), which rules out "the road is too
narrow" and points at missing or wrong block shapes.

With only 8 usable fitting cases the physics constants are not identifiable: a 150-generation CEM
fit drives top speed down to 116 km/h to match a handful of short maps, and makes the corpus worse.
`replayfit` therefore exists but its output is not adopted automatically; `rti experiment
'{"kind":"replay_bench"}'` is the standing accuracy measurement inside the research loop.

### What would move the number

1. **Block shape coverage.** Loops, wall rides, diagonal roads, the `*Special*` gameplay blocks and
   the many transition variants are approximated as straights or not modelled at all. Each family
   added to `shape3d.rs` removes a class of early failures.
2. **Closed-loop evaluation.** Human replays carry inputs but no positions, so error compounds.
   Comparing checkpoint split times with the car constrained to the road (a rail mode) would make
   the speed profile identifiable without needing the exact line.
3. **Real-game verification.** The oracle bridge (`docs/oracle.md`) replays inputs in TM2020 and
   returns per-tick positions; that turns this from an inference problem into a measurement.


## Surfaces

TM2020 blocks are built from materials that handle very differently, so `Surface` covers all of
them rather than four: asphalt (concrete/tech), dirt, grass, ice, bump, plastic, water, sand,
snow, metal (track walls and wall rides) and penalty. `PhysicsParams` holds a grip, a drive and a
drag multiplier **per surface** (`surface_grip`, `surface_drive`, `surface_drag`), all in the
flat parameter vector so calibration fits them independently. The catalog maps family tokens to
materials in order, so `RoadIce` is ice, `PlatformPlastic` is plastic, `RoadBump` is bump,
`RoadWater` is water, `SnowRoad` is snow and `DesertRoad` is sand.

## Two routers, and how routes are judged without physics

`replaybench` measures the car; it cannot tell a physics error from a wrong route. `routecheck`
separates the two using only the replay's checkpoint split times: for a correct route the fraction
of the distance reached at checkpoint *k* tracks the fraction of the time elapsed, and a route
that takes a wrong branch or mirrors a turn diverges. It involves no simulation at all.

Measured over the 126-replay corpus:

| router | routes with a complete start-to-finish route and checkpoints | deviation under 0.05 |
|---|---|---|
| block chaining (default) | 5 | 2 |
| surface routing (`--routed`) | 6 | 0 |

`compile_track_routed` is the second one: rasterise every block we can shape, give the rest of the
drivable families a flat patch, then take the shortest path over the heightfield from the start
block through the checkpoints to the finish. It removes the "one unmodelled block breaks the
chain" failure but currently produces worse routes, so block chaining stays the default. Its
remaining problems are known: 28 maps have no recognised start block, and 47 have no connected
surface path from the start.

## What was ruled out

Each of these was a plausible cause of the low finish rate and was tested and eliminated:

* **Steering sign.** Dropping TM2020's vertical axis mirrors the plane, so the sign was suspect.
  Flipping it drops finishes from 8 to 3, so the current convention is right.
* **Corridor too narrow.** Widening the guaranteed-drivable corridor by 8 m and by 20 m does not
  raise the finish count.
* **Holes from unmodelled blocks.** Giving every unshaped block of a drivable family a flat patch
  changes nothing measurable.

The remaining causes, from the stop-reason breakdown of the 60 usable runs (fell 22, stuck on
walls 19, finished 8, tick limit 6, inputs exhausted 5), are wrong routes and wrong block shapes.

## Why "exact" needs the game

Two sources of ground truth exist and both are currently closed:

* **The game's own collision meshes.** `Packs/Stadium.pak` in the install is an encrypted NadeoPak
  (version 18), so block shapes cannot be read off disk.
* **Measurement through Openplanet.** The bridge plugin is written and installed
  (`deploy/openplanet/RTIBridge`), but the Openplanet binary states that full permissions require
  the Club Edition of Trackmania, which is what unsigned plugins need. `DeveloperMode=true` is now
  set in the prefix's `Settings.ini` (backed up alongside) in case that is enough on its own.

Until one of those opens, every geometry error is only visible indirectly, through the two
instruments above.

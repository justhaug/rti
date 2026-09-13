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

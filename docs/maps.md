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

* Block direction `d` rotates the block `d × 90°` counter-clockwise in the (x, z) plane about the
  centre of its origin cell (`compile::rot` with sign +1).
* A `RoadTech*Curve1` at direction 0 joins its **top** edge (+z) to its **left** edge (−x), arc
  centred on the footprint's top-left corner. Larger `CurveN` blocks are assumed to be the same
  quarter circle scaled to N×N cells.
* Straights and markers (start / checkpoint / finish / multilap) run through the cell along z.

Not yet verified: the origin cell of rotated multi-cell blocks (the compiler tries both the
origin-cell and bounding-box conventions and keeps whichever chains more road), the yaw sign of
free blocks (also tried both ways), chicane handedness, and slope/tilt footprints.

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
* Replay/ghost ingestion (TMX replays, Nadeo record ghosts) as trajectories in the archive so
  search can start from human state of the art.
* Bulk ingestion (`search TMX → download N maps → compile → classify`) as a scheduled task.

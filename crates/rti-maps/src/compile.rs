//! Chain block placements into a planar `Track`.

use std::collections::HashMap;

use rti_core::track::TrackNode;
use rti_core::{Surface, Track};
use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Marker, Shape, CELL};
use crate::gbx::{MapBlock, ParsedMap};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CompileReport {
    pub blocks_total: usize,
    pub blocks_recognized: usize,
    pub blocks_chained: usize,
    pub unrecognized: Vec<(String, usize)>,
    pub start_found: bool,
    pub finish_found: bool,
    pub checkpoints: usize,
    /// Runs of unrecognised blocks bridged with straight segments.
    pub bridged_gaps: usize,
    pub length_m: f32,
    pub convention: String,
    pub warnings: Vec<String>,
}

/// A placed road piece in world (planar) coordinates.
#[derive(Clone, Debug)]
struct Piece {
    #[allow(dead_code)]
    idx: usize,
    entry: (f32, f32),
    exit: (f32, f32),
    /// centerline points from entry to exit (inclusive)
    path: Vec<(f32, f32)>,
    half_width: f32,
    surface: Surface,
    marker: Option<Marker>,
}

/// Local template geometry: (entry, exit, path) in the block's own frame.
fn local_geometry(shape: Shape, len: u32, size: u32, shift: i32) -> LocalGeometry {
    let h = CELL / 2.0;
    match shape {
        Shape::Straight | Shape::Marker => {
            let l = len.max(1) as f32 * CELL;
            ((h, 0.0), (h, l), vec![(h, 0.0), (h, l * 0.5), (h, l)])
        }
        Shape::Curve => {
            // Verified on real maps: a Curve at direction 0 joins the TOP edge
            // (v = n*CELL, last... first column) to the LEFT edge (u = 0), arc
            // centred on the top-left corner of the footprint.
            let n = size.max(1) as f32;
            let r = n * CELL - h; // centerline radius
            let cx = 0.0;
            let cy = n * CELL;
            let steps = (6.0 * n) as usize;
            let mut path = vec![];
            for i in 0..=steps {
                // from angle 0 (point (r, cy) on the top edge) down to -90° (point (0, cy - r) on the left edge)
                let a = -(i as f32 / steps as f32) * std::f32::consts::FRAC_PI_2;
                path.push((cx + r * a.cos(), cy + r * a.sin()));
            }
            ((r, cy), (0.0, cy - r), path)
        }
        Shape::Chicane => {
            let l = len.max(2) as f32 * CELL;
            let du = -(shift as f32) * CELL; // shift left = -u in a right-handed local frame
            let steps = 12;
            let mut path = vec![];
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let s = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
                path.push((h + du * s, l * t));
            }
            ((h, 0.0), (h + du, l), path)
        }
    }
}

/// Footprint in local cell units (i along u, j along v).
fn footprint(shape: Shape, len: u32, size: u32, shift: i32) -> (i32, i32, i32) {
    // returns (min_i, width_i, len_j)
    match shape {
        Shape::Straight | Shape::Marker => (0, 1, len.max(1) as i32),
        Shape::Curve => (0, size.max(1) as i32, size.max(1) as i32),
        Shape::Chicane => {
            if shift > 0 {
                (-1, 2, len.max(2) as i32)
            } else {
                (0, 2, len.max(2) as i32)
            }
        }
    }
}

/// Placement conventions the compiler tries. The rotation sign for grid
/// blocks was verified on real maps (+1 with `rot`); the origin convention
/// for multi-cell blocks and the yaw sign of free blocks are still tried.
#[derive(Clone, Copy, Debug)]
struct Convention {
    /// +1: dir rotates counter-clockwise in (x, z); -1: clockwise.
    sign: i32,
    /// whether the map coord is the rotated origin cell (true) or the
    /// bounding-box minimum (false).
    origin_cell: bool,
    /// yaw sign for free blocks
    yaw_sign: f32,
}

fn rot(p: (f32, f32), dir: u8, sign: i32) -> (f32, f32) {
    let q = ((dir as i32 * sign).rem_euclid(4)) as u8;
    match q {
        0 => p,
        1 => (-p.1, p.0),
        2 => (-p.0, -p.1),
        _ => (p.1, -p.0),
    }
}

fn place(block: &MapBlock, idx: usize, cat: &Catalog, conv: Convention) -> Option<Piece> {
    let res = cat.resolve(&block.name)?;
    let t = &res.template;
    let (entry, exit, path) = local_geometry(t.shape, t.len, t.size, t.shift);
    if block.free {
        // free block: absolute origin + yaw about the vertical axis
        let pos = block.free_pos?;
        let yaw = block.free_pyr.map(|p| p[1]).unwrap_or(0.0) * conv.yaw_sign;
        let (c, s) = (yaw.cos(), yaw.sin());
        let to_world = move |p: (f32, f32)| -> (f32, f32) {
            (pos[0] + c * p.0 - s * p.1, pos[2] + s * p.0 + c * p.1)
        };
        return Some(Piece {
            idx,
            entry: to_world(entry),
            exit: to_world(exit),
            path: path.into_iter().map(to_world).collect(),
            half_width: t.half_width,
            surface: res.surface,
            marker: t.marker,
        });
    }
    let (min_i, w, l) = footprint(t.shape, t.len, t.size, t.shift);
    // local geometry lives in cells [min_i, min_i+w) × [0, l); shift so that
    // the origin cell is the pivot
    let pivot = (CELL / 2.0, CELL / 2.0);
    let to_world = |p: (f32, f32)| -> (f32, f32) {
        let local = (p.0 - pivot.0, p.1 - pivot.1);
        let r = rot(local, block.dir, conv.sign);
        let (ox, oz) = if conv.origin_cell {
            (block.coord[0] as f32 * CELL, block.coord[2] as f32 * CELL)
        } else {
            // coord is the min corner of the rotated bounding box: compute
            // the rotated footprint extent and offset so its min is at coord
            let corners = [
                rot(
                    (min_i as f32 * CELL - pivot.0, -pivot.1),
                    block.dir,
                    conv.sign,
                ),
                rot(
                    ((min_i + w) as f32 * CELL - pivot.0, -pivot.1),
                    block.dir,
                    conv.sign,
                ),
                rot(
                    (min_i as f32 * CELL - pivot.0, l as f32 * CELL - pivot.1),
                    block.dir,
                    conv.sign,
                ),
                rot(
                    (
                        (min_i + w) as f32 * CELL - pivot.0,
                        l as f32 * CELL - pivot.1,
                    ),
                    block.dir,
                    conv.sign,
                ),
            ];
            let minx = corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
            let minz = corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min);
            (
                block.coord[0] as f32 * CELL - minx - pivot.0,
                block.coord[2] as f32 * CELL - minz - pivot.1,
            )
        };
        (ox + pivot.0 + r.0, oz + pivot.1 + r.1)
    };
    Some(Piece {
        idx,
        entry: to_world(entry),
        exit: to_world(exit),
        path: path.into_iter().map(to_world).collect(),
        half_width: t.half_width,
        surface: res.surface,
        marker: t.marker,
    })
}

fn key(p: (f32, f32)) -> (i32, i32) {
    ((p.0 / 4.0).round() as i32, (p.1 / 4.0).round() as i32)
}

/// Chain pieces from a start marker; pieces can be traversed in either
/// direction (entry↔exit) since block direction conventions are uncertain.
/// (pieces, chain sequence, reached finish, convention description, bridged gaps)
type Candidate = (Vec<Piece>, Vec<(usize, bool)>, bool, String, usize);
/// (entry, exit, path) of a template in its local frame.
type LocalGeometry = ((f32, f32), (f32, f32), Vec<(f32, f32)>);

/// Unrecognised blocks between two recognised ones are bridged with a
/// straight segment up to this many cells long.
const MAX_GAP_CELLS: usize = 3;

fn chain(pieces: &[Piece]) -> (Vec<(usize, bool)>, bool, usize) {
    let mut best_gaps = 0usize;
    let mut by_point: HashMap<(i32, i32), Vec<(usize, bool)>> = HashMap::new();
    for (i, p) in pieces.iter().enumerate() {
        by_point.entry(key(p.entry)).or_default().push((i, false)); // enter via entry, forward
        by_point.entry(key(p.exit)).or_default().push((i, true)); // enter via exit, reversed
    }
    let starts: Vec<usize> = pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.marker, Some(Marker::Start | Marker::StartFinish)))
        .map(|(i, _)| i)
        .collect();
    let mut best: Vec<(usize, bool)> = vec![];
    let mut best_finished = false;
    let candidates: Vec<(usize, bool)> = if starts.is_empty() {
        (0..pieces.len())
            .flat_map(|i| [(i, false), (i, true)])
            .collect()
    } else {
        starts
            .iter()
            .flat_map(|&i| [(i, false), (i, true)])
            .collect()
    };
    for (s, rev) in candidates {
        let mut used = vec![false; pieces.len()];
        let mut gaps: Vec<(usize, usize)> = vec![];
        let mut seq = vec![(s, rev)];
        used[s] = true;
        let mut finished = matches!(pieces[s].marker, Some(Marker::Finish));
        loop {
            let (cur, r) = *seq.last().unwrap();
            let p = &pieces[cur];
            let out = if r { p.entry } else { p.exit };
            // direction of travel at the exit, for bridging small gaps
            let (a, b) = if r {
                (p.path[1.min(p.path.len() - 1)], p.path[0])
            } else {
                (p.path[p.path.len() - 2], p.path[p.path.len() - 1])
            };
            let d = ((b.0 - a.0), (b.1 - a.1));
            let dl = (d.0 * d.0 + d.1 * d.1).sqrt().max(1e-3);
            let dir = (d.0 / dl, d.1 / dl);
            let mut next = by_point
                .get(&key(out))
                .and_then(|v| v.iter().find(|(j, _)| !used[*j]).copied());
            let mut gap = 0usize;
            if next.is_none() {
                for cells in 1..=MAX_GAP_CELLS {
                    let probe = (
                        out.0 + dir.0 * CELL * cells as f32,
                        out.1 + dir.1 * CELL * cells as f32,
                    );
                    if let Some(n) = by_point
                        .get(&key(probe))
                        .and_then(|v| v.iter().find(|(j, _)| !used[*j]).copied())
                    {
                        next = Some(n);
                        gap = cells;
                        break;
                    }
                }
            }
            match next {
                Some((j, jr)) => {
                    used[j] = true;
                    if gap > 0 {
                        gaps.push((seq.len(), gap));
                    }
                    seq.push((j, jr));
                    if matches!(pieces[j].marker, Some(Marker::Finish | Marker::StartFinish)) {
                        finished = true;
                        break;
                    }
                }
                None => break,
            }
        }
        if (finished && !best_finished) || (finished == best_finished && seq.len() > best.len()) {
            best = seq;
            best_finished = finished;
            best_gaps = gaps.len();
        }
    }
    (best, best_finished, best_gaps)
}

pub fn compile_track(
    map: &ParsedMap,
    cat: &Catalog,
    name: &str,
) -> anyhow::Result<(Track, CompileReport)> {
    let mut report = CompileReport {
        blocks_total: map.blocks.len(),
        ..Default::default()
    };
    let mut unrec: HashMap<String, usize> = HashMap::new();
    for b in &map.blocks {
        if cat.resolve(&b.name).is_some() {
            report.blocks_recognized += 1;
        } else {
            *unrec.entry(b.name.clone()).or_default() += 1;
        }
    }
    let mut u: Vec<(String, usize)> = unrec.into_iter().collect();
    u.sort_by_key(|a| std::cmp::Reverse(a.1));
    u.truncate(30);
    report.unrecognized = u;
    anyhow::ensure!(
        report.blocks_recognized > 0,
        "no recognised road blocks in map (catalog coverage 0)"
    );

    let mut best: Option<Candidate> = None;
    let has_free = map.blocks.iter().any(|b| b.free && b.free_pos.is_some());
    let yaw_signs: &[f32] = if has_free { &[1.0, -1.0] } else { &[1.0] };
    for origin_cell in [true, false] {
        for &yaw_sign in yaw_signs {
            let conv = Convention {
                sign: 1,
                origin_cell,
                yaw_sign,
            };
            let pieces: Vec<Piece> = map
                .blocks
                .iter()
                .enumerate()
                .filter_map(|(i, b)| place(b, i, cat, conv))
                .collect();
            let (seq, finished, gaps) = chain(&pieces);
            let desc = format!("origin_cell={origin_cell} yaw_sign={yaw_sign}");
            let better = match &best {
                None => true,
                Some((_, bseq, bfin, _, _)) => {
                    (finished && !bfin) || (finished == *bfin && seq.len() > bseq.len())
                }
            };
            if better {
                best = Some((pieces, seq, finished, desc, gaps));
            }
        }
    }
    let (pieces, seq, finished, desc, gaps) = best.unwrap();
    report.convention = desc;
    report.bridged_gaps = gaps;
    report.blocks_chained = seq.len();
    report.finish_found = finished;
    report.start_found = matches!(
        pieces[seq[0].0].marker,
        Some(Marker::Start | Marker::StartFinish)
    );
    if !report.start_found {
        report
            .warnings
            .push("no start block recognised; chain begins at an arbitrary piece".into());
    }
    if !finished {
        report.warnings.push(
            "chain did not reach a finish block; track ends where the road could not be followed"
                .into(),
        );
    }

    // build nodes
    let mut nodes: Vec<TrackNode> = vec![];
    let mut checkpoints = vec![];
    let mut finish = None;
    for (i, &(pi, rev)) in seq.iter().enumerate() {
        let p = &pieces[pi];
        let mut path = p.path.clone();
        if rev {
            path.reverse();
        }
        let start_idx = nodes.len();
        for (k, &(x, y)) in path.iter().enumerate() {
            if i > 0 && k == 0 {
                continue; // shared with the previous piece's last point
            }
            if let Some(last) = nodes.last() {
                if (last.x - x).abs() < 1e-3 && (last.y - y).abs() < 1e-3 {
                    continue;
                }
            }
            nodes.push(TrackNode {
                x,
                y,
                half_width: p.half_width,
                surface: p.surface,
            });
        }
        let mid = (start_idx + nodes.len().saturating_sub(1)) / 2;
        match p.marker {
            Some(Marker::Checkpoint) => checkpoints.push(mid.max(start_idx)),
            Some(Marker::Finish) | Some(Marker::StartFinish) if i > 0 => {
                finish = Some(mid.max(start_idx))
            }
            _ => {}
        }
    }
    anyhow::ensure!(nodes.len() >= 2, "compiled track has fewer than two nodes");
    let mut track = Track {
        name: name.to_string(),
        description: format!(
            "Imported from TM2020 map {:?} by {} ({} blocks, {} chained, convention {})",
            map.info.name,
            map.info.author_nick,
            map.blocks.len(),
            seq.len(),
            report.convention
        ),
        nodes,
        checkpoints,
        finish,
        max_ticks: 12_000,
        tm_map_uid: Some(map.info.uid.clone()),
        tm_frame: None,
        walls: true,
    };
    track.checkpoints.sort_unstable();
    track.checkpoints.dedup();
    if let Some(f) = track.finish {
        track.checkpoints.retain(|&c| c < f);
    }
    // author time gives a sensible tick cap
    if map.info.author_ms > 0 {
        track.max_ticks = ((map.info.author_ms / 10) * 3).max(3000);
    }
    report.checkpoints = track.checkpoints.len();
    report.length_m = track.length();
    track.validate()?;
    Ok((track, report))
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!(
            "{}…",
            &s[..s
                .char_indices()
                .map(|(i, _)| i)
                .find(|&i| i >= n)
                .unwrap_or(n)]
        )
    }
}

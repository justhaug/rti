//! Chain block placements into a planar `Track`.
//!
//! Every recognised block becomes a `Piece` with a footprint and a set of
//! ports (edge midpoints in world coordinates). Two-port pieces (straights,
//! curves, chicanes, markers) have one path; open pieces (platforms) can be
//! crossed straight or with a quarter turn between any two edges. The
//! track is the cheapest port-to-port path from a start marker to a finish
//! marker (Dijkstra; straight continuations are cheaper than turns, small
//! gaps of unrecognised cells are bridged at a cost). Without a reachable
//! finish, the longest reachable chain is used.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

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

type P2 = (f32, f32);

/// A placed piece in world (planar) coordinates.
#[derive(Clone, Debug)]
struct Piece {
    ports: Vec<P2>,
    /// outward unit direction at each port
    dirs: Vec<P2>,
    /// world position of the piece centre (for open-piece paths)
    centre: P2,
    /// paths for two-port pieces: from port 0 to port 1
    fixed_path: Option<Vec<P2>>,
    half_width: f32,
    surface: Surface,
    marker: Option<Marker>,
    y: f32,
}

/// Rotate a local point by `dir` quarter turns (counter-clockwise, verified).
fn rot(p: P2, dir: u8) -> P2 {
    match dir & 3 {
        0 => p,
        1 => (-p.1, p.0),
        2 => (-p.0, -p.1),
        _ => (p.1, -p.0),
    }
}

/// Local geometry: ports, outward directions, optional fixed path.
fn local(
    shape: Shape,
    len: u32,
    size: u32,
    shift: i32,
) -> (Vec<P2>, Vec<P2>, Option<Vec<P2>>, (i32, i32, i32)) {
    let h = CELL / 2.0;
    match shape {
        Shape::Straight | Shape::Marker => {
            let l = len.max(1) as f32 * CELL;
            (
                vec![(h, 0.0), (h, l)],
                vec![(0.0, -1.0), (0.0, 1.0)],
                Some(vec![(h, 0.0), (h, l * 0.5), (h, l)]),
                (0, 1, len.max(1) as i32),
            )
        }
        Shape::Curve => {
            // Verified on real maps (Curve1/2/3 at several rotations, with the
            // bounding-box placement rule): footprint N×N from the origin
            // cell, road enters through the top edge of the last column and
            // leaves through the left edge of the first row; arc centred on
            // the footprint's top-left corner.
            let n = size.max(1) as f32;
            let r = n * CELL - h;
            let cy = n * CELL;
            let steps = (6.0 * n) as usize;
            let mut path = vec![];
            for i in 0..=steps {
                let a = -(i as f32 / steps as f32) * std::f32::consts::FRAC_PI_2;
                path.push((r * a.cos(), cy + r * a.sin()));
            }
            (
                vec![(r, cy), (0.0, cy - r)],
                vec![(0.0, 1.0), (-1.0, 0.0)],
                Some(path),
                (0, size.max(1) as i32, size.max(1) as i32),
            )
        }
        Shape::Chicane => {
            // Verified: a "Right" chicane shifts toward negative u.
            let l = len.max(2) as f32 * CELL;
            let du = shift as f32 * CELL;
            let steps = 12;
            let mut path = vec![];
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let s = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
                path.push((h + du * s, l * t));
            }
            let fp = if shift < 0 {
                (-1, 2, len.max(2) as i32)
            } else {
                (0, 2, len.max(2) as i32)
            };
            (
                vec![(h, 0.0), (h + du, l)],
                vec![(0.0, -1.0), (0.0, 1.0)],
                Some(path),
                fp,
            )
        }
        Shape::Open => {
            let w = size.max(1) as f32 * CELL;
            let l = len.max(1) as f32 * CELL;
            // ports: bottom, top, left, right (midpoints); multi-cell edges get one port per cell
            let mut ports = vec![];
            let mut dirs = vec![];
            for i in 0..size.max(1) {
                let u = (i as f32 + 0.5) * CELL;
                ports.push((u, 0.0));
                dirs.push((0.0, -1.0));
                ports.push((u, l));
                dirs.push((0.0, 1.0));
            }
            for j in 0..len.max(1) {
                let v = (j as f32 + 0.5) * CELL;
                ports.push((0.0, v));
                dirs.push((-1.0, 0.0));
                ports.push((w, v));
                dirs.push((1.0, 0.0));
            }
            (
                ports,
                dirs,
                None,
                (0, size.max(1) as i32, len.max(1) as i32),
            )
        }
    }
}

fn place(block: &MapBlock, cat: &Catalog, origin_cell: bool, yaw_sign: f32) -> Option<Piece> {
    let res = cat.resolve(&block.name)?;
    let t = &res.template;
    let (ports, dirs, path, (min_i, w, l)) = local(t.shape, t.len, t.size, t.shift);
    let pivot = (CELL / 2.0, CELL / 2.0);
    let (to_world, to_world_dir): (Box<dyn Fn(P2) -> P2>, Box<dyn Fn(P2) -> P2>) = if block.free {
        let pos = block.free_pos?;
        let yaw = block.free_pyr.map(|p| p[1]).unwrap_or(0.0) * yaw_sign;
        let (c, s) = (yaw.cos(), yaw.sin());
        (
            Box::new(move |p: P2| (pos[0] + c * p.0 - s * p.1, pos[2] + s * p.0 + c * p.1)),
            Box::new(move |d: P2| (c * d.0 - s * d.1, s * d.0 + c * d.1)),
        )
    } else {
        let dir = block.dir;
        let (ox, oz) = if origin_cell {
            (block.coord[0] as f32 * CELL, block.coord[2] as f32 * CELL)
        } else {
            let corners = [
                rot((min_i as f32 * CELL - pivot.0, -pivot.1), dir),
                rot(((min_i + w) as f32 * CELL - pivot.0, -pivot.1), dir),
                rot(
                    (min_i as f32 * CELL - pivot.0, l as f32 * CELL - pivot.1),
                    dir,
                ),
                rot(
                    (
                        (min_i + w) as f32 * CELL - pivot.0,
                        l as f32 * CELL - pivot.1,
                    ),
                    dir,
                ),
            ];
            let minx = corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
            let minz = corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min);
            (
                block.coord[0] as f32 * CELL - minx - pivot.0,
                block.coord[2] as f32 * CELL - minz - pivot.1,
            )
        };
        (
            Box::new(move |p: P2| {
                let r = rot((p.0 - pivot.0, p.1 - pivot.1), dir);
                (ox + pivot.0 + r.0, oz + pivot.1 + r.1)
            }),
            Box::new(move |d: P2| rot(d, dir)),
        )
    };
    let centre = to_world((
        (min_i as f32 + w as f32 / 2.0) * CELL,
        l as f32 * CELL / 2.0,
    ));
    Some(Piece {
        ports: ports.into_iter().map(|p| to_world(p)).collect(),
        dirs: dirs.into_iter().map(|d| to_world_dir(d)).collect(),
        centre,
        fixed_path: path.map(|p| p.into_iter().map(|q| to_world(q)).collect()),
        half_width: t.half_width,
        surface: res.surface,
        marker: t.marker,
        y: block
            .free_pos
            .map(|p| p[1])
            .unwrap_or(block.coord[1] as f32 * 8.0),
    })
}

fn key(p: P2) -> (i32, i32) {
    ((p.0 / 4.0).round() as i32, (p.1 / 4.0).round() as i32)
}

/// Path through piece `pi` entering at port `a`, leaving at port `b`.
fn path_through(p: &Piece, a: usize, b: usize) -> Vec<P2> {
    if let Some(fp) = &p.fixed_path {
        let mut v = fp.clone();
        if a == 1 {
            v.reverse();
        }
        return v;
    }
    let pa = p.ports[a];
    let pb = p.ports[b];
    let da = p.dirs[a];
    let db = p.dirs[b];
    let straight = (da.0 * db.0 + da.1 * db.1) < -0.5;
    if straight {
        vec![pa, ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0), pb]
    } else {
        // quarter arc via the corner region: bezier-ish through the centre
        let c = p.centre;
        let mut v = vec![];
        for i in 0..=8 {
            let t = i as f32 / 8.0;
            let x = (1.0 - t).powi(2) * pa.0 + 2.0 * (1.0 - t) * t * c.0 + t * t * pb.0;
            let y = (1.0 - t).powi(2) * pa.1 + 2.0 * (1.0 - t) * t * c.1 + t * t * pb.1;
            v.push((x, y));
        }
        v
    }
}

const MAX_GAP_CELLS: usize = 3;
/// Distance from the start block's entry edge to the car's spawn point.
pub const START_SPAWN_M: f32 = 30.0;

#[derive(Clone, Copy, PartialEq)]
struct QItem {
    cost: f32,
    piece: usize,
    entry: usize,
}
impl Eq for QItem {}
impl Ord for QItem {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.partial_cmp(&self.cost).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for QItem {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Result of one Dijkstra stage: path of (piece, entry, exit) ending at a
/// target piece (its exit chosen as the port opposite the entry), or the
/// deepest reachable state if no target was reached.
struct Stage {
    seq: Vec<(usize, usize, usize)>,
    reached: bool,
    gaps: usize,
    /// state to continue from: (piece, exit port)
    end: (usize, usize),
}

fn opposite_port(p: &Piece, entry: usize) -> usize {
    if p.ports.len() == 2 {
        1 - entry
    } else {
        (0..p.ports.len())
            .find(|&x| {
                x != entry && (p.dirs[x].0 * p.dirs[entry].0 + p.dirs[x].1 * p.dirs[entry].1) < -0.5
            })
            .unwrap_or(entry)
    }
}

/// Dijkstra over (piece, entry port) states. `seeds` are (piece, entry)
/// states already "inside" a piece; `target` marks pieces that end the stage
/// (entered through port 0 when they have two ports); `blocked` pieces are
/// not traversed.
fn dijkstra(
    pieces: &[Piece],
    by_point: &HashMap<(i32, i32), Vec<(usize, usize)>>,
    seeds: &[(usize, usize)],
    target: &dyn Fn(usize) -> bool,
    blocked: &std::collections::HashSet<usize>,
    require_back_entry: bool,
) -> Stage {
    let idx = |piece: usize, entry: usize| piece * 16 + entry;
    let mut dist: HashMap<usize, f32> = HashMap::new();
    let mut prev: HashMap<usize, (usize, usize, usize)> = HashMap::new();
    let mut gapc: HashMap<usize, usize> = HashMap::new();
    let mut heap = BinaryHeap::new();
    for &(p, e) in seeds {
        dist.insert(idx(p, e), 0.0);
        heap.push(QItem {
            cost: 0.0,
            piece: p,
            entry: e,
        });
    }
    let seed_set: std::collections::HashSet<usize> = seeds.iter().map(|s| s.0).collect();
    let mut goal: Option<(usize, usize)> = None;
    let mut deepest: (f32, usize, usize) = (-1.0, seeds[0].0, seeds[0].1);
    let mut visited = std::collections::HashSet::new();
    while let Some(QItem { cost, piece, entry }) = heap.pop() {
        let st = idx(piece, entry);
        if !visited.insert(st) {
            continue;
        }
        if cost > deepest.0 {
            deepest = (cost, piece, entry);
        }
        if !seed_set.contains(&piece)
            && target(piece)
            && (!require_back_entry || pieces[piece].ports.len() != 2 || entry == 0)
        {
            goal = Some((piece, entry));
            break;
        }
        let p = &pieces[piece];
        for exit in 0..p.ports.len().min(16) {
            if exit == entry {
                continue;
            }
            if p.fixed_path.is_some() && p.ports.len() == 2 && exit != 1 - entry {
                continue;
            }
            let d_in = p.dirs[entry];
            let d_out = p.dirs[exit];
            let turn = if (d_in.0 * d_out.0 + d_in.1 * d_out.1) < -0.5 {
                0.0
            } else {
                0.6
            };
            let out = p.ports[exit];
            let mut targets: Vec<((usize, usize), f32, usize)> = vec![];
            for cells in 0..=MAX_GAP_CELLS {
                let probe = (
                    out.0 + d_out.0 * CELL * cells as f32,
                    out.1 + d_out.1 * CELL * cells as f32,
                );
                if let Some(v) = by_point.get(&key(probe)) {
                    for &(j, k) in v {
                        if j == piece || blocked.contains(&j) {
                            continue;
                        }
                        let dj = pieces[j].dirs[k];
                        if dj.0 * d_out.0 + dj.1 * d_out.1 > -0.5 {
                            continue;
                        }
                        let dy = (pieces[j].y - p.y).abs();
                        if dy > 24.0 {
                            continue;
                        }
                        targets.push(((j, k), 1.0 + turn + cells as f32 * 2.0 + dy * 0.2, cells));
                    }
                    if !targets.is_empty() {
                        break;
                    }
                }
            }
            for ((j, k), c, cells) in targets {
                let nst = idx(j, k);
                let nc = cost + c;
                if nc < *dist.get(&nst).unwrap_or(&f32::INFINITY) {
                    dist.insert(nst, nc);
                    prev.insert(nst, (piece, entry, exit));
                    gapc.insert(nst, if cells > 0 { 1 } else { 0 });
                    heap.push(QItem {
                        cost: nc,
                        piece: j,
                        entry: k,
                    });
                }
            }
        }
    }
    let (end_piece, end_entry, reached) = match goal {
        Some((p, e)) => (p, e, true),
        None => (deepest.1, deepest.2, false),
    };
    let last_exit = opposite_port(&pieces[end_piece], end_entry);
    let mut seq: Vec<(usize, usize, usize)> = vec![(end_piece, end_entry, last_exit)];
    let mut gaps = 0usize;
    let mut cur = (end_piece, end_entry);
    let mut guard = 0;
    while let Some(&(pp, pe, px)) = prev.get(&idx(cur.0, cur.1)) {
        gaps += gapc.get(&idx(cur.0, cur.1)).copied().unwrap_or(0);
        seq.push((pp, pe, px));
        cur = (pp, pe);
        guard += 1;
        if guard > 100_000 {
            break;
        }
    }
    seq.reverse();
    Stage {
        seq,
        reached,
        gaps,
        end: (end_piece, last_exit),
    }
}

struct Chain {
    /// (piece, entry port, exit port)
    seq: Vec<(usize, usize, usize)>,
    finished: bool,
    gaps: usize,
    score: (u32, u32),
}

/// Chain from a seed: if it is a start block, route through every reachable
/// checkpoint (greedy nearest-next) and then to a finish; otherwise route to
/// a finish or as deep as possible.
fn chain_from(
    pieces: &[Piece],
    by_point: &HashMap<(i32, i32), Vec<(usize, usize)>>,
    s: usize,
    checkpoints: &[usize],
) -> Chain {
    let is_finish =
        |i: usize| matches!(pieces[i].marker, Some(Marker::Finish | Marker::StartFinish));
    let seed_is_start = matches!(pieces[s].marker, Some(Marker::Start | Marker::StartFinish));
    // seed states: a start block is left through its top port (entered "from" the bottom)
    let seeds: Vec<(usize, usize)> = if seed_is_start && pieces[s].ports.len() == 2 {
        vec![(s, 0)]
    } else {
        (0..pieces[s].ports.len().min(16)).map(|e| (s, e)).collect()
    };
    let mut blocked: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut seq: Vec<(usize, usize, usize)> = vec![];
    let mut gaps = 0;
    let mut cur_seeds = seeds;
    let mut remaining: Vec<usize> = if seed_is_start {
        checkpoints.to_vec()
    } else {
        vec![]
    };
    let mut finished = false;
    loop {
        let targets_left = !remaining.is_empty();
        let target = |i: usize| {
            if targets_left {
                remaining.contains(&i)
            } else {
                is_finish(i)
            }
        };
        let st = dijkstra(
            pieces,
            by_point,
            &cur_seeds,
            &target,
            &blocked,
            !targets_left,
        );
        // append (skip the seed piece if it is already the tail of seq)
        let skip = if seq.is_empty() { 0 } else { 1 };
        for &(p, a, b) in st.seq.iter().skip(skip) {
            seq.push((p, a, b));
            if pieces[p].fixed_path.is_some() {
                blocked.insert(p);
            }
        }
        if seq.is_empty() {
            seq = st.seq.clone();
        }
        gaps += st.gaps;
        if !st.reached {
            if targets_left && seq.len() > 1 {
                // no checkpoint reachable from here: give up on the remaining
                // checkpoints and head for the finish from where we are
                remaining.clear();
                let (p, a, _b) = *seq.last().unwrap();
                cur_seeds = vec![(p, a)];
                let st2 = dijkstra(
                    pieces,
                    by_point,
                    &cur_seeds,
                    &|i| is_finish(i),
                    &blocked,
                    true,
                );
                for &(pp, aa, bb) in st2.seq.iter().skip(1) {
                    seq.push((pp, aa, bb));
                }
                gaps += st2.gaps;
                finished = st2.reached;
            }
            break;
        }
        if targets_left {
            remaining.retain(|&c| c != st.end.0);
            cur_seeds = vec![(st.end.0, opposite_port(&pieces[st.end.0], st.end.1))];
            // continue from the checkpoint: we entered at `entry`, leave via `exit`
            let (p, a, _b) = *seq.last().unwrap();
            cur_seeds = vec![(p, a)];
            let _ = st.end;
            if remaining.is_empty() && seq.len() > 1 {
                // now route to the finish from the checkpoint we are in
                continue;
            }
            continue;
        }
        finished = true;
        break;
    }
    let from_start = seed_is_start;
    let metres: f32 = seq
        .iter()
        .map(|&(pi, a, b)| {
            let path = path_through(&pieces[pi], a, b);
            path.windows(2)
                .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
                .sum::<f32>()
        })
        .sum();
    let score = (
        0u32,
        (metres + if finished { 300.0 } else { 0.0 } + if from_start { 100.0 } else { 0.0 }) as u32,
    );
    if std::env::var("RTI_CHAIN_DEBUG").is_ok() {
        let p0 = pieces[s].ports[0];
        eprintln!("seed {s} at ({:.0},{:.0}) marker {:?}: pieces {} metres {:.0} finished {finished} score {:?}", p0.0, p0.1, pieces[s].marker, seq.len(), metres, score);
    }
    Chain {
        seq,
        finished,
        gaps,
        score,
    }
}

fn chain(pieces: &[Piece]) -> Chain {
    let mut by_point: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    for (i, p) in pieces.iter().enumerate() {
        for (k, &pt) in p.ports.iter().enumerate() {
            by_point.entry(key(pt)).or_default().push((i, k));
        }
    }
    let n = pieces.len();
    let starts: Vec<usize> = pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.marker, Some(Marker::Start | Marker::StartFinish)))
        .map(|(i, _)| i)
        .collect();
    let checkpoints: Vec<usize> = pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.marker, Some(Marker::Checkpoint)))
        .map(|(i, _)| i)
        .collect();
    let mut seeds: Vec<usize> = starts.clone();
    for (i, p) in pieces.iter().enumerate() {
        if seeds.len() > 4000 {
            break;
        }
        let dead_end = p.ports.iter().enumerate().any(|(k, pt)| {
            let d = p.dirs[k];
            let probe = (pt.0 + d.0 * CELL, pt.1 + d.1 * CELL);
            let any = |q: P2| {
                by_point
                    .get(&key(q))
                    .map(|v| v.iter().any(|(j, _)| *j != i))
                    .unwrap_or(false)
            };
            !any(*pt) && !any(probe)
        });
        if dead_end && !seeds.contains(&i) {
            seeds.push(i);
        }
    }
    let candidates: Vec<usize> = if seeds.is_empty() {
        (0..n).collect()
    } else {
        seeds
    };
    let mut best: Option<Chain> = None;
    for s in candidates {
        let c = chain_from(pieces, &by_point, s, &checkpoints);
        let better = match &best {
            None => true,
            Some(b) => c.score > b.score,
        };
        if better {
            best = Some(c);
        }
    }
    best.unwrap_or(Chain {
        seq: vec![],
        finished: false,
        gaps: 0,
        score: (0, 0),
    })
}

pub fn compile_track(
    map: &ParsedMap,
    cat: &Catalog,
    name: &str,
) -> anyhow::Result<(Track, CompileReport)> {
    compile_track_with(map, cat, name, None)
}

/// Like `compile_track`, optionally forcing the multi-cell origin convention
/// (`Some(true)` = origin cell, `Some(false)` = bounding-box minimum).
pub fn compile_track_with(
    map: &ParsedMap,
    cat: &Catalog,
    name: &str,
    force_origin_cell: Option<bool>,
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
        "no recognised drivable blocks in map (catalog coverage 0)"
    );

    let has_free = map.blocks.iter().any(|b| b.free && b.free_pos.is_some());
    let yaw_signs: &[f32] = if has_free { &[1.0, -1.0] } else { &[1.0] };
    let mut best: Option<(Vec<Piece>, Chain, String)> = None;
    // Placement rule (verified on real maps): the block coordinate is the
    // minimum corner of the rotated footprint's bounding box.
    let conventions: Vec<bool> = match force_origin_cell {
        Some(v) => vec![v],
        None => vec![false],
    };
    for origin_cell in conventions {
        for &yaw_sign in yaw_signs {
            let pieces: Vec<Piece> = map
                .blocks
                .iter()
                .filter_map(|b| place(b, cat, origin_cell, yaw_sign))
                .collect();
            if pieces.len() > 60_000 {
                anyhow::bail!("too many drivable pieces ({})", pieces.len());
            }
            let ch = chain(&pieces);
            let desc = format!("origin_cell={origin_cell} yaw_sign={yaw_sign}");
            let better = match &best {
                None => true,
                Some((_, b, _)) => {
                    (ch.finished && !b.finished)
                        || (ch.finished == b.finished && ch.seq.len() > b.seq.len())
                }
            };
            if better {
                best = Some((pieces, ch, desc));
            }
        }
    }
    let (pieces, ch, desc) = best.unwrap();
    report.convention = desc;
    report.blocks_chained = ch.seq.len();
    report.finish_found = ch.finished;
    report.bridged_gaps = ch.gaps;
    anyhow::ensure!(!ch.seq.is_empty(), "could not chain any piece");
    report.start_found = matches!(
        pieces[ch.seq[0].0].marker,
        Some(Marker::Start | Marker::StartFinish)
    );
    if !report.start_found {
        report
            .warnings
            .push("no start block recognised; chain begins at an arbitrary piece".into());
    }
    if !ch.finished {
        report.warnings.push(
            "chain did not reach a finish block; track ends where the road could not be followed"
                .into(),
        );
    }

    let mut nodes: Vec<TrackNode> = vec![];
    let mut checkpoints = vec![];
    let mut finish = None;
    for (i, &(pi, a, b)) in ch.seq.iter().enumerate() {
        let p = &pieces[pi];
        let path = path_through(p, a, b);
        let start_idx = nodes.len();
        for (k, &(x, y)) in path.iter().enumerate() {
            if i > 0 && k == 0 {
                if let Some(last) = nodes.last() {
                    // bridge gap with a straight segment (last node → this port)
                    if (last.x - x).abs() > 1e-3 || (last.y - y).abs() > 1e-3 {
                        nodes.push(TrackNode {
                            x,
                            y,
                            half_width: p.half_width,
                            surface: p.surface,
                        });
                    }
                }
                continue;
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
    // The car spawns near the exit edge of the start block (about 2 m before
    // the start line, measured on tiny maps with known replay times); race
    // distance is counted from there. Trim the polyline accordingly.
    if report.start_found {
        let mut cut = START_SPAWN_M;
        let mut i = 0;
        while i + 1 < nodes.len() {
            let seg = ((nodes[i + 1].x - nodes[i].x).powi(2)
                + (nodes[i + 1].y - nodes[i].y).powi(2))
            .sqrt();
            if cut < seg {
                let t = cut / seg;
                let a = nodes[i];
                let b = nodes[i + 1];
                nodes[i] = TrackNode {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                    half_width: a.half_width,
                    surface: a.surface,
                };
                break;
            }
            cut -= seg;
            i += 1;
        }
        if i > 0 && i < nodes.len() {
            nodes.drain(0..i);
            for c in checkpoints.iter_mut() {
                *c = c.saturating_sub(i);
            }
            if let Some(f) = finish.as_mut() {
                *f = f.saturating_sub(i);
            }
        }
    }
    let mut track = Track {
        name: name.to_string(),
        description: format!(
            "Imported from TM2020 map {:?} by {} ({} blocks, {} chained, convention {})",
            map.info.name,
            map.info.author_nick,
            map.blocks.len(),
            ch.seq.len(),
            report.convention
        ),
        nodes,
        checkpoints,
        finish,
        max_ticks: 12_000,
        tm_map_uid: Some(map.info.uid.clone()),
        tm_map_file: Some(format!("RTI/{name}.Map.Gbx")),
        tm_frame: None,
        walls: true,
    };
    track.checkpoints.sort_unstable();
    track.checkpoints.dedup();
    if let Some(f) = track.finish {
        track.checkpoints.retain(|&c| c < f);
    }
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

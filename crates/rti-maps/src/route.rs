//! Routing over the rasterised drivable surface.
//!
//! Port-to-port chaining breaks at every block whose shape we do not model.
//! The heightfield does not care about block shapes: it knows which ground is
//! drivable and at what height. Routing is therefore a shortest path over the
//! grid of surface samples, from the start block through every checkpoint
//! block to the finish block. Unmodelled blocks contribute their flat
//! fallback patch and the path simply crosses them.

use std::collections::BinaryHeap;

use rti_sim::world::{Layer, World, RES};

/// A node of the surface graph: a grid cell plus which layer of it.
pub type NodeId = (i32, i32, u8);
type Node = NodeId;

fn cell_of(w: &World, x: f32, z: f32) -> (i32, i32) {
    (
        ((x / RES).floor() as i32) - w.x0,
        ((z / RES).floor() as i32) - w.z0,
    )
}

fn layers_at(w: &World, gx: i32, gz: i32) -> Vec<Layer> {
    if gx < 0 || gz < 0 || gx as usize >= w.w || gz as usize >= w.h {
        return vec![];
    }
    let i = gz as usize * w.w + gx as usize;
    w.layers[w.offsets[i] as usize..w.offsets[i + 1] as usize].to_vec()
}

/// Largest height step between neighbouring cells that a route may cross.
pub const MAX_STEP_M: f32 = 9.0;

#[derive(Clone, Copy, PartialEq)]
struct Item {
    cost: f32,
    node: Node,
}
impl Eq for Item {}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.cost
            .partial_cmp(&self.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

pub struct Surface<'a> {
    pub world: &'a World,
}

impl<'a> Surface<'a> {
    pub fn new(world: &'a World) -> Surface<'a> {
        Surface { world }
    }

    fn height(&self, n: Node) -> Option<f32> {
        layers_at(self.world, n.0, n.1)
            .get(n.2 as usize)
            .map(|l| l.y)
    }

    /// Nodes of the cells covered by `piece`. Layers are merged when patches
    /// overlap, so a piece's own id can be overwritten; cells within
    /// `radius` of `centre` at a comparable height are included as well.
    pub fn nodes_of_piece(&self, piece: u32, centre: (f32, f32, f32), radius: f32) -> Vec<Node> {
        let mut out = vec![];
        let (cx, cz) = cell_of(self.world, centre.0, centre.1);
        let r = (radius / RES).ceil() as i32;
        for gz in (cz - r)..=(cz + r) {
            for gx in (cx - r)..=(cx + r) {
                for (k, l) in layers_at(self.world, gx, gz).iter().enumerate() {
                    let near = (((gx - cx) * (gx - cx) + (gz - cz) * (gz - cz)) as f32).sqrt()
                        * RES
                        <= radius
                        && (l.y - centre.2).abs() <= MAX_STEP_M;
                    if l.piece == piece || near {
                        out.push((gx, gz, k as u8));
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Shortest path over the surface from any of `from` to any of `to`.
    /// Returns the path in world coordinates with heights.
    pub fn path(&self, from: &[Node], to: &[Node]) -> Option<Vec<(f32, f32, f32)>> {
        if from.is_empty() || to.is_empty() {
            return None;
        }
        let goal: std::collections::HashSet<Node> = to.iter().copied().collect();
        let mut dist: std::collections::HashMap<Node, f32> = Default::default();
        let mut prev: std::collections::HashMap<Node, Node> = Default::default();
        let mut heap = BinaryHeap::new();
        for &n in from {
            if self.height(n).is_some() {
                dist.insert(n, 0.0);
                heap.push(Item { cost: 0.0, node: n });
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut reached = None;
        while let Some(Item { cost, node }) = heap.pop() {
            if !seen.insert(node) {
                continue;
            }
            if goal.contains(&node) {
                reached = Some(node);
                break;
            }
            let Some(y) = self.height(node) else { continue };
            for dz in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let (nx, nz) = (node.0 + dx, node.1 + dz);
                    for (k, l) in layers_at(self.world, nx, nz).iter().enumerate() {
                        let dy = (l.y - y).abs();
                        if dy > MAX_STEP_M {
                            continue;
                        }
                        let step = ((dx * dx + dz * dz) as f32).sqrt() * RES;
                        let nn = (nx, nz, k as u8);
                        // prefer smooth ground, but allow the drops and jumps real routes contain
                        let nc = cost + step + dy * dy * 0.5;
                        if nc < *dist.get(&nn).unwrap_or(&f32::INFINITY) {
                            dist.insert(nn, nc);
                            prev.insert(nn, node);
                            heap.push(Item { cost: nc, node: nn });
                        }
                    }
                }
            }
        }
        let mut node = reached?;
        let mut path = vec![];
        loop {
            let y = self.height(node).unwrap_or(0.0);
            path.push((
                (node.0 + self.world.x0) as f32 * RES + RES / 2.0,
                (node.1 + self.world.z0) as f32 * RES + RES / 2.0,
                y,
            ));
            match prev.get(&node) {
                Some(&p) => node = p,
                None => break,
            }
        }
        path.reverse();
        Some(path)
    }
}

/// Thin a dense grid path to points at least `step` metres apart, keeping
/// direction changes.
pub fn simplify(path: &[(f32, f32, f32)], step: f32) -> Vec<(f32, f32, f32)> {
    let mut out: Vec<(f32, f32, f32)> = vec![];
    for &p in path {
        match out.last() {
            Some(&q) if ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt() < step => {}
            _ => out.push(p),
        }
    }
    if let Some(&last) = path.last() {
        if out.last() != Some(&last) {
            out.push(last);
        }
    }
    out
}

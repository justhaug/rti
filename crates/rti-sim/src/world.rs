//! Layered heightfield world: the 3D drivable surface of a map.
//!
//! Every drivable block is rasterised into `Patch`es (height, surface,
//! drivable mask, wall mask) at `RES` metres per sample. Cells may hold
//! several layers (overpasses); queries pick the layer nearest to the car's
//! current height. This is what the 3D physics drives on; the centerline
//! polyline is only used for race progress.

use rti_core::Surface;
use serde::{Deserialize, Serialize};

pub const RES: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// surface height (m)
    pub y: f32,
    pub surface: u8,
    /// 0 = road piece with side walls, 1 = open surface (fall off the edge)
    pub kind: u8,
    /// piece id (index into the compiler's piece list)
    pub piece: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
pub struct Sample {
    pub y: f32,
    /// unit normal (x, y, z) of the surface
    pub normal: [f32; 3],
    pub surface: Surface,
    pub open: bool,
    pub piece: u32,
}

/// A rasterised block: a local grid of layers over a world-space bounding box.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Patch {
    pub x0: i32,
    pub z0: i32,
    pub w: usize,
    pub h: usize,
    /// row-major, `None` = not drivable at that sample
    pub cells: Vec<Option<Layer>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct World {
    pub x0: i32,
    pub z0: i32,
    pub w: usize,
    pub h: usize,
    /// CSR layout: `layers[offsets[i]..offsets[i+1]]` for cell i
    pub offsets: Vec<u32>,
    pub layers: Vec<Layer>,
}

impl World {
    pub fn from_patches(patches: &[Patch]) -> World {
        if patches.is_empty() {
            return World::default();
        }
        let x0 = patches.iter().map(|p| p.x0).min().unwrap() - 4;
        let z0 = patches.iter().map(|p| p.z0).min().unwrap() - 4;
        let x1 = patches.iter().map(|p| p.x0 + p.w as i32).max().unwrap() + 4;
        let z1 = patches.iter().map(|p| p.z0 + p.h as i32).max().unwrap() + 4;
        let w = (x1 - x0) as usize;
        let h = (z1 - z0) as usize;
        let mut cells: Vec<Vec<Layer>> = vec![Vec::new(); w * h];
        for p in patches {
            for j in 0..p.h {
                for i in 0..p.w {
                    if let Some(l) = p.cells[j * p.w + i] {
                        let gx = (p.x0 + i as i32 - x0) as usize;
                        let gz = (p.z0 + j as i32 - z0) as usize;
                        let c = &mut cells[gz * w + gx];
                        // merge layers closer than 1.5 m (keep the higher one: later pieces overlay)
                        if let Some(existing) = c.iter_mut().find(|e| (e.y - l.y).abs() < 1.5) {
                            if l.y >= existing.y {
                                *existing = l;
                            }
                        } else {
                            c.push(l);
                        }
                    }
                }
            }
        }
        let mut offsets = Vec::with_capacity(w * h + 1);
        let mut layers = Vec::new();
        offsets.push(0u32);
        for c in cells {
            layers.extend(c);
            offsets.push(layers.len() as u32);
        }
        World {
            x0,
            z0,
            w,
            h,
            offsets,
            layers,
        }
    }

    #[inline]
    fn cell_layers(&self, gx: i32, gz: i32) -> &[Layer] {
        if gx < 0 || gz < 0 || gx as usize >= self.w || gz as usize >= self.h {
            return &[];
        }
        let i = gz as usize * self.w + gx as usize;
        &self.layers[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    /// Layer at world (x, z) nearest to height `y` (within `tol` metres).
    #[inline]
    pub fn layer_at(&self, x: f32, z: f32, y: f32, tol: f32) -> Option<Layer> {
        let gx = ((x / RES).floor() as i32) - self.x0;
        let gz = ((z / RES).floor() as i32) - self.z0;
        let mut best: Option<Layer> = None;
        for l in self.cell_layers(gx, gz) {
            let d = (l.y - y).abs();
            if d <= tol && best.map(|b| d < (b.y - y).abs()).unwrap_or(true) {
                best = Some(*l);
            }
        }
        best
    }

    /// Surface sample with a finite-difference normal. Returns None when
    /// nothing drivable is within `tol` of `y`.
    pub fn sample(&self, x: f32, z: f32, y: f32, tol: f32) -> Option<Sample> {
        let l = self.layer_at(x, z, y, tol)?;
        let h = |dx: f32, dz: f32| {
            self.layer_at(x + dx, z + dz, l.y, 3.0)
                .map(|q| q.y)
                .unwrap_or(l.y)
        };
        let d = RES;
        let dydx = (h(d, 0.0) - h(-d, 0.0)) / (2.0 * d);
        let dydz = (h(0.0, d) - h(0.0, -d)) / (2.0 * d);
        let n = [-dydx, 1.0, -dydz];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        Some(Sample {
            y: l.y,
            normal: [n[0] / len, n[1] / len, n[2] / len],
            surface: surface_from(l.surface),
            open: l.kind == 1,
            piece: l.piece,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

pub fn surface_from(i: u8) -> Surface {
    match i {
        1 => Surface::Dirt,
        2 => Surface::Grass,
        3 => Surface::Ice,
        _ => Surface::Asphalt,
    }
}

/// Helper to build a patch from a closure over world-space sample centres.
pub fn rasterize(
    x_min: f32,
    z_min: f32,
    x_max: f32,
    z_max: f32,
    f: impl Fn(f32, f32) -> Option<Layer>,
) -> Patch {
    let x0 = (x_min / RES).floor() as i32;
    let z0 = (z_min / RES).floor() as i32;
    let x1 = (x_max / RES).ceil() as i32;
    let z1 = (z_max / RES).ceil() as i32;
    let w = (x1 - x0).max(1) as usize;
    let h = (z1 - z0).max(1) as usize;
    let mut cells = Vec::with_capacity(w * h);
    for j in 0..h {
        for i in 0..w {
            let x = (x0 + i as i32) as f32 * RES + RES / 2.0;
            let z = (z0 + j as i32) as f32 * RES + RES / 2.0;
            cells.push(f(x, z));
        }
    }
    Patch {
        x0,
        z0,
        w,
        h,
        cells,
    }
}

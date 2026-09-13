//! 3D block templates: height, drivable mask and surface for a piece in its
//! local frame (u to the right, v forward, cells of 32 m). Parameters come
//! from the block-name tokens (slope units, tilt direction, start/end
//! transitions). The exact TM2020 profiles are unknown; these are smooth
//! approximations meant to be calibrated against replays.

use crate::catalog::{tokens, Resolved, Shape, CELL};

/// Block height unit (metres).
pub const UNIT_H: f32 = 8.0;
/// Bank angle of "Tilt" pieces (radians).
pub const TILT_RAD: f32 = 0.45;
/// Half width of road surfaces (metres).
pub const ROAD_HW: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    Flat,
    /// linear rise of `rise` metres over the piece length
    Slope,
    /// flat → slope (cosine ease-in)
    SlopeStart,
    /// slope → flat (cosine ease-out)
    SlopeEnd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bank {
    None,
    /// constant bank, +1 = right side up
    Full(f32),
    /// bank goes from 0 to `sign` over the piece (v)
    In(f32),
    /// bank goes from `sign` to 0
    Out(f32),
}

#[derive(Clone, Copy, Debug)]
pub struct Template3 {
    pub shape: Shape,
    pub len: u32,
    pub size: u32,
    pub shift: i32,
    /// footprint (min_i, width cells, length cells)
    pub fp: (i32, i32, i32),
    pub profile: Profile,
    /// total rise over the piece (metres), sign = up along +v
    pub rise: f32,
    pub bank: Bank,
    pub half_width: f32,
    pub open: bool,
    /// height of the piece's bottom/entry edge relative to its block base (mined)
    pub base_shift: f32,
}

pub fn template3(name: &str, r: &Resolved) -> Template3 {
    let toks = tokens(name);
    let has = |t: &str| toks.iter().any(|x| x == t);
    let t = &r.template;
    let fp = match t.shape {
        Shape::Straight | Shape::Marker => (0, 1, t.len.max(1) as i32),
        Shape::Curve => (0, t.size.max(1) as i32, t.size.max(1) as i32),
        Shape::Chicane => {
            if t.shift < 0 {
                (-1, 2, t.len.max(2) as i32)
            } else {
                (0, 2, t.len.max(2) as i32)
            }
        }
        Shape::Open => (0, t.size.max(1) as i32, t.len.max(1) as i32),
    };
    // slope units from tokens: "Slope" = 1 unit per cell, "Slope2" = 2 units per cell
    let mut units_per_cell = 0.0f32;
    let mut profile = Profile::Flat;
    let _ = &mut profile;
    for tok in &toks {
        if let Some(rest) = tok.strip_prefix("Slope") {
            let n: f32 = rest
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(1.0);
            units_per_cell = n.max(1.0);
            profile = Profile::Slope;
            if rest.contains("Start") || has("Start") && !has("Finish") && has("Slope") {
                profile = Profile::SlopeStart;
            }
            if rest.contains("End") || has("End") {
                profile = Profile::SlopeEnd;
            }
            if rest.contains("Base") || has("Base") {
                profile = Profile::Flat;
                units_per_cell = 0.0;
            }
        }
    }
    // "NxM" tokens on slope pieces: N cells long rising M units in total
    let mut rise = units_per_cell * UNIT_H * fp.2 as f32;
    for tok in &toks {
        if let Some((a, b)) = crate::catalog::dims_pub(tok) {
            if profile != Profile::Flat {
                rise = b as f32 * UNIT_H;
                let _ = a;
            }
        }
    }
    if matches!(profile, Profile::SlopeStart | Profile::SlopeEnd)
        && !toks.iter().any(|t| crate::catalog::dims_pub(t).is_some())
    {
        // transition pieces rise half a slope unit
        rise *= 0.5;
    }
    if has("Down") && !has("Tilt") {
        rise = -rise;
    }
    // banking
    let sign = if has("Left") { -1.0 } else { 1.0 };
    let bank = if has("Tilt") {
        if has("Transition") {
            if has("Up") {
                Bank::In(sign)
            } else {
                Bank::Out(sign)
            }
        } else {
            // banked curves: outside (right, for our left-turning curves) is higher
            Bank::Full(if t.shape == Shape::Curve { 1.0 } else { sign })
        }
    } else {
        Bank::None
    };
    // mined data overrides the name heuristics when available
    let mut base_shift = 0.0;
    let off = |k: usize| {
        r.port_offsets
            .iter()
            .find(|(p, _)| *p == k)
            .map(|(_, v)| *v)
    };
    if let (Some(o0), Some(o1)) = (off(0), off(1)) {
        base_shift = o0;
        let mined_rise = o1 - o0;
        if t.shape != Shape::Open {
            if mined_rise.abs() > 0.5 && profile == Profile::Flat {
                profile = Profile::Slope;
            }
            if profile != Profile::Flat || mined_rise.abs() > 0.5 {
                rise = mined_rise;
            }
        }
    } else if let Some(o0) = off(0) {
        base_shift = o0;
    }
    Template3 {
        shape: t.shape,
        len: t.len.max(1),
        size: t.size.max(1),
        shift: t.shift,
        fp,
        profile,
        rise,
        bank,
        base_shift,
        half_width: if t.shape == Shape::Open || r.template.half_width > 10.0 {
            16.0
        } else {
            ROAD_HW
        },
        open: t.shape == Shape::Open,
    }
}

impl Template3 {
    pub fn length_m(&self) -> f32 {
        self.fp.2 as f32 * CELL
    }

    /// Height relative to the piece base at local (u, v); None if not drivable.
    pub fn height(&self, u: f32, v: f32) -> Option<f32> {
        let l = self.length_m();
        let w = self.fp.1 as f32 * CELL;
        let u_min = self.fp.0 as f32 * CELL;
        if v < 0.0 || v > l || u < u_min || u > u_min + w {
            return None;
        }
        // lateral offset from the drivable centre / drivable test
        let (lateral, drivable) = match self.shape {
            Shape::Straight | Shape::Marker => {
                let c = CELL / 2.0;
                ((u - c), (u - c).abs() <= self.half_width)
            }
            Shape::Curve => {
                let n = self.size as f32;
                let r = n * CELL - CELL / 2.0;
                let cy = n * CELL;
                let d = ((u - 0.0).powi(2) + (v - cy).powi(2)).sqrt();
                (
                    (d - r),
                    (d - r).abs() <= self.half_width && u >= 0.0 && v <= cy,
                )
            }
            Shape::Chicane => {
                let du = self.shift as f32 * CELL;
                let t = (v / l).clamp(0.0, 1.0);
                let s = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
                let centre = CELL / 2.0 + du * s;
                ((u - centre), (u - centre).abs() <= self.half_width)
            }
            Shape::Open => (u - u_min - w / 2.0, true),
        };
        if !drivable {
            return None;
        }
        let t = (v / l).clamp(0.0, 1.0);
        let mut h = self.base_shift
            + match self.profile {
                Profile::Flat => 0.0,
                Profile::Slope => self.rise * t,
                Profile::SlopeStart => self.rise * (1.0 - (std::f32::consts::FRAC_PI_2 * t).cos()),
                Profile::SlopeEnd => self.rise * (std::f32::consts::FRAC_PI_2 * t).sin(),
            };
        let bank = match self.bank {
            Bank::None => 0.0,
            Bank::Full(s) => s * TILT_RAD,
            Bank::In(s) => s * TILT_RAD * t,
            Bank::Out(s) => s * TILT_RAD * (1.0 - t),
        };
        if bank != 0.0 {
            // right side (positive lateral) higher for positive bank
            h += bank.tan() * lateral;
        }
        Some(h)
    }
}

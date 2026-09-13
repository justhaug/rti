//! Block asset catalog: maps vanilla block names to planar geometry.
//!
//! Resolution is grammar based: a TM2020 block name is CamelCase tokens
//! (`RoadTechTiltTransition2UpRightCurveIn` → Road Tech Tilt Transition2 Up
//! Right Curve In). The family tokens decide the surface, the shape tokens
//! decide geometry and footprint. Anything decorative or non-drivable
//! (walls, pillars, loops, deco, land, water, trees) resolves to nothing.
//! A project can add regex overrides in `catalog.toml`; they take
//! precedence over the grammar.
//!
//! Local frame of a piece (direction 0): cells are 32 m; `u` grows to the
//! right, `v` forward. Verified on real maps: a `Curve` joins its top edge
//! (+v) to its left edge (−u); straights run along v.

use regex::Regex;
use rti_core::Surface;
use serde::{Deserialize, Serialize};

pub const CELL: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// `len` cells long, straight through (two ports).
    Straight,
    /// Quarter turn spanning `size`×`size` cells, top edge ↔ left edge.
    Curve,
    /// `len` cells long, exits shifted `shift` cells (+ = left).
    Chicane,
    /// Straight with a race marker.
    Marker,
    /// Open drivable surface (platforms): ports on all four edges of a
    /// `size`×`len` footprint; may be crossed straight or with a turn.
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Marker {
    Start,
    Finish,
    StartFinish,
    Checkpoint,
    Multilap,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Template {
    /// Regex on the block name (overrides only).
    pub pattern: String,
    pub shape: Shape,
    #[serde(default = "one")]
    pub len: u32,
    #[serde(default = "one")]
    pub size: u32,
    #[serde(default)]
    pub shift: i32,
    #[serde(default)]
    pub marker: Option<Marker>,
    #[serde(default = "default_hw")]
    pub half_width: f32,
    #[serde(default)]
    pub surface: Option<Surface>,
    #[serde(default)]
    pub note: String,
}

fn one() -> u32 {
    1
}
fn default_hw() -> f32 {
    8.0
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CatalogFile {
    #[serde(default)]
    pub template: Vec<Template>,
    #[serde(default)]
    pub surfaces: Vec<(String, Surface)>,
    /// Extra name tokens that mark a block as non-drivable.
    #[serde(default)]
    pub ignore_tokens: Vec<String>,
}

pub struct Catalog {
    pub overrides: Vec<(Regex, Template)>,
    pub surfaces: Vec<(String, Surface)>,
    pub ignore_tokens: Vec<String>,
}

/// Resolved geometry for one block name.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub template: Template,
    pub surface: Surface,
    pub family: String,
    /// How the name was resolved ("override" | "grammar").
    pub via: &'static str,
}

pub const DEFAULT_CATALOG: &str = r#"
# Overrides (regex on the full block name) take precedence over the grammar.
# Family token → surface; the first matching token wins.
surfaces = [
  ["Ice", "ice"], ["Snow", "dirt"], ["Dirt", "dirt"], ["Grass", "grass"], ["Rally", "dirt"],
  ["Tech", "asphalt"], ["Bump", "asphalt"], ["Plastic", "asphalt"], ["Desert", "asphalt"], ["Water", "asphalt"],
]
ignore_tokens = ["Wall", "Pillar", "Loop", "Deadend", "Structure", "Land", "Tree", "Cliff", "Beach",
  "Canopy", "Stand", "Fence", "Sign", "Screen", "Light", "Trigger", "Arrow", "Pipe", "Tunnel",
  "Border", "Support", "Column", "Roof", "Flag", "Lamp", "Cover", "Gap", "Hole", "Bridge",
  "Antenna", "Panel", "Tower", "Barrier", "Cactus", "Rock", "Bush", "Bubble", "Bar"]
"#;

/// Split CamelCase (digits kept with the preceding word: "Transition2").
pub fn tokens(name: &str) -> Vec<String> {
    let name = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let name = name.split('.').next().unwrap_or(name);
    let mut out: Vec<String> = vec![];
    let mut cur = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let boundary = c.is_ascii_uppercase()
            && i > 0
            && !(chars[i - 1].is_ascii_uppercase()
                && chars
                    .get(i + 1)
                    .map(|n| n.is_ascii_uppercase() || !n.is_ascii_alphabetic())
                    .unwrap_or(true));
        if boundary && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if c == '_' || c == '-' || c == ' ' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn leading_number(tok: &str, prefix: &str) -> Option<u32> {
    tok.strip_prefix(prefix)?
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

/// Public wrapper for `dims`.
pub fn dims_pub(tok: &str) -> Option<(u32, u32)> {
    dims(tok)
}

/// Trailing "2x1" style dimensions in a token → (a, b).
fn dims(tok: &str) -> Option<(u32, u32)> {
    let b: Vec<u8> = tok.bytes().collect();
    let mut i = b.len();
    while i > 0 && b[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == b.len() || i == 0 || (b[i - 1] != b'x' && b[i - 1] != b'X') {
        return None;
    }
    let second: u32 = tok[i..].parse().ok()?;
    let mut j = i - 1;
    while j > 0 && b[j - 1].is_ascii_digit() {
        j -= 1;
    }
    if j == i - 1 {
        return None;
    }
    let first: u32 = tok[j..i - 1].parse().ok()?;
    Some((first, second))
}

impl Catalog {
    pub fn parse(toml_text: &str) -> anyhow::Result<Catalog> {
        let file: CatalogFile = toml::from_str(toml_text)?;
        let mut overrides = vec![];
        for t in file.template {
            let re = Regex::new(&t.pattern)
                .map_err(|e| anyhow::anyhow!("bad catalog pattern {:?}: {e}", t.pattern))?;
            overrides.push((re, t));
        }
        Ok(Catalog {
            overrides,
            surfaces: file.surfaces,
            ignore_tokens: file.ignore_tokens,
        })
    }

    pub fn default_catalog() -> Catalog {
        Self::parse(DEFAULT_CATALOG).expect("embedded catalog is valid")
    }

    /// Default catalog plus optional overrides from `<root>/catalog.toml`.
    pub fn load(root: &std::path::Path) -> anyhow::Result<Catalog> {
        let mut cat = Self::default_catalog();
        let p = root.join("catalog.toml");
        if p.exists() {
            let extra = Self::parse(&std::fs::read_to_string(&p)?)?;
            let mut t = extra.overrides;
            t.extend(cat.overrides);
            cat.overrides = t;
            let mut s = extra.surfaces;
            s.extend(cat.surfaces);
            cat.surfaces = s;
            cat.ignore_tokens.extend(extra.ignore_tokens);
        }
        Ok(cat)
    }

    pub fn surface_for_tokens(&self, toks: &[String]) -> Surface {
        for (fam, s) in &self.surfaces {
            if toks.iter().any(|t| t == fam) {
                return *s;
            }
        }
        Surface::Asphalt
    }

    pub fn resolve(&self, name: &str) -> Option<Resolved> {
        for (re, t) in &self.overrides {
            if re.is_match(name) {
                let toks = tokens(name);
                let surface = t.surface.unwrap_or_else(|| self.surface_for_tokens(&toks));
                return Some(Resolved {
                    template: t.clone(),
                    surface,
                    family: toks.first().cloned().unwrap_or_default(),
                    via: "override",
                });
            }
        }
        self.resolve_grammar(name)
    }

    fn resolve_grammar(&self, name: &str) -> Option<Resolved> {
        let toks = tokens(name);
        if toks.is_empty() {
            return None;
        }
        let has = |t: &str| toks.iter().any(|x| x == t);
        let starts = |p: &str| toks.iter().any(|x| x.starts_with(p));
        if toks
            .iter()
            .any(|t| self.ignore_tokens.iter().any(|i| t == i))
        {
            return None;
        }
        // drivable families: roads, platforms, gates, decorative platforms and hills
        // (hills are drivable dirt/grass mounds used constantly in dirt maps), water ramps
        let deco_drivable = toks[0] == "Deco" && (has("Platform") || has("Hill"));
        let road = has("Road")
            || has("Platform")
            || has("Gate")
            || deco_drivable
            || (has("Ramp") && has("Road"));
        if !road {
            return None;
        }
        // diagonal road pieces have geometry we do not model; diagonal platform
        // pieces are treated as open surfaces below
        if has("Road") && starts("Diag") {
            return None;
        }
        // markers first
        let slope = starts("Slope") || starts("Tilt") || starts("Transition");
        let marker = if has("Start") && has("Finish") || has("StartFinish") {
            Some(Marker::StartFinish)
        } else if (has("Start") || starts("Start")) && !slope {
            Some(Marker::Start)
        } else if has("Finish") {
            Some(Marker::Finish)
        } else if has("Checkpoint") {
            Some(Marker::Checkpoint)
        } else if has("Multilap") {
            Some(Marker::Multilap)
        } else {
            None
        };
        let mut surface = self.surface_for_tokens(&toks);
        if has("Hill")
            && !toks
                .iter()
                .any(|t| ["Ice", "Dirt", "Grass", "Snow"].contains(&t.as_str()))
        {
            surface = Surface::Dirt;
        }
        let family = toks
            .iter()
            .take_while(|t| !is_shape_token(t))
            .cloned()
            .collect::<Vec<_>>()
            .join("");
        let mut t = Template {
            pattern: String::new(),
            shape: Shape::Straight,
            len: 1,
            size: 1,
            shift: 0,
            marker: None,
            half_width: 8.0,
            surface: Some(surface),
            note: String::new(),
        };
        // footprint hints
        let mut len_hint: Option<u32> = None;
        for tok in &toks {
            if let Some(n) = leading_number(tok, "X") {
                len_hint = Some(n);
            }
            if let Some((a, _b)) = dims(tok) {
                len_hint = Some(a.max(1));
            }
        }
        if has("Platform") {
            t.half_width = 16.0;
        }
        if let Some(m) = marker {
            t.shape = Shape::Marker;
            t.marker = Some(m);
            t.len = len_hint.unwrap_or(1);
            return Some(Resolved {
                template: t,
                surface,
                family,
                via: "grammar",
            });
        }
        if let Some(n) = toks.iter().find_map(|x| leading_number(x, "Curve")) {
            if has("Chicane") {
                // chicane variants named ...ChicaneX2Left
            } else {
                t.shape = Shape::Curve;
                t.size = n.clamp(1, 6);
                return Some(Resolved {
                    template: t,
                    surface,
                    family,
                    via: "grammar",
                });
            }
        }
        // "...CurveIn"/"...CurveOut" transitions and unnumbered curves are quarter turns
        if has("Curve") && !has("Chicane") {
            t.shape = Shape::Curve;
            t.size = 1;
            return Some(Resolved {
                template: t,
                surface,
                family,
                via: "grammar",
            });
        }
        if has("Chicane") {
            t.shape = Shape::Chicane;
            t.len = len_hint.unwrap_or(2).max(2);
            t.shift = if has("Left") { 1 } else { -1 };
            return Some(Resolved {
                template: t,
                surface,
                family,
                via: "grammar",
            });
        }
        // junctions: any edge to any edge
        if has("Branch") || has("Cross") {
            t.shape = Shape::Open;
            return Some(Resolved {
                template: t,
                surface,
                family,
                via: "grammar",
            });
        }
        // open platform surfaces: ...Base, ...Base2x2, diagonal platform pieces; slopes are directional
        if (has("Platform") || toks[0] == "Deco")
            && (has("Base")
                || starts("Diag")
                || toks.last().map(|l| dims(l).is_some()).unwrap_or(false))
            && !starts("Slope")
            && !has("Curve")
        {
            t.shape = Shape::Open;
            let (a, b) = toks.iter().find_map(|x| dims(x)).unwrap_or((1, 1));
            t.size = a.max(1);
            t.len = b.max(1);
            return Some(Resolved {
                template: t,
                surface,
                family,
                via: "grammar",
            });
        }
        // straights and everything that behaves like one
        let straight_like = has("Straight")
            || has("Slope")
            || has("Tilt")
            || has("Transition")
            || has("Ramp")
            || has("Turbo")
            || has("Special")
            || has("Boost")
            || has("Reset")
            || has("Fragile")
            || has("Cruise")
            || has("Engine")
            || has("Brake")
            || has("Steering")
            || has("Motion")
            || has("Narrow")
            || has("To")
            || starts("Slope")
            || starts("Tilt");
        if straight_like {
            t.shape = Shape::Straight;
            t.len = len_hint.unwrap_or(1);
            return Some(Resolved {
                template: t,
                surface,
                family,
                via: "grammar",
            });
        }
        None
    }
}

fn is_shape_token(t: &str) -> bool {
    let shapes = [
        "Straight",
        "Curve",
        "Chicane",
        "Start",
        "Finish",
        "Checkpoint",
        "Multilap",
        "Slope",
        "Tilt",
        "Transition",
        "Base",
        "Turbo",
        "Special",
        "Boost",
        "Reset",
        "Fragile",
        "Cruise",
        "Narrow",
        "Diag",
        "Loop",
        "Wall",
        "To",
    ];
    shapes.iter().any(|s| t.starts_with(s))
        || t.chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
}

/// Write a track TOML into a tracks directory.
pub fn write_track(dir: &std::path::Path, track: &rti_core::Track) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        dir.join(format!("{}.toml", track.name)),
        toml::to_string_pretty(track)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenises_names() {
        assert_eq!(
            tokens("RoadTechTiltTransition2UpRightCurveIn"),
            vec![
                "Road",
                "Tech",
                "Tilt",
                "Transition2",
                "Up",
                "Right",
                "Curve",
                "In"
            ]
        );
        assert_eq!(
            tokens("PlatformTechBase2x2"),
            vec!["Platform", "Tech", "Base2x2"]
        );
        assert_eq!(
            tokens("zzz_ImportedItems\\Magnet_Blocks\\M2_PlatformTechBase.Block.Gbx_CustomBlock"),
            vec!["M2", "Platform", "Tech", "Base"]
        );
    }

    #[test]
    fn resolves_common_names() {
        let c = Catalog::default_catalog();
        assert_eq!(
            c.resolve("RoadTechStraight").unwrap().template.shape,
            Shape::Straight
        );
        let cv = c.resolve("RoadTechCurve2").unwrap();
        assert_eq!(cv.template.shape, Shape::Curve);
        assert_eq!(cv.template.size, 2);
        assert_eq!(c.resolve("RoadDirtCurve1").unwrap().surface, Surface::Dirt);
        assert_eq!(
            c.resolve("RoadTechStart").unwrap().template.marker,
            Some(Marker::Start)
        );
        assert_eq!(
            c.resolve("RoadTechFinish").unwrap().template.marker,
            Some(Marker::Finish)
        );
        assert_eq!(
            c.resolve("RoadTechCheckpointTiltRight")
                .unwrap()
                .template
                .marker,
            Some(Marker::Checkpoint)
        );
        assert_eq!(
            c.resolve("RoadIceCheckpoint").unwrap().surface,
            Surface::Ice
        );
        let ch = c.resolve("RoadTechChicaneX3TiltRight").unwrap();
        assert_eq!(ch.template.shape, Shape::Chicane);
        assert_eq!((ch.template.len, ch.template.shift), (3, -1));
        assert!(c.resolve("DecoWallBasePillar").is_none());
        assert!(c.resolve("StructurePillar").is_none());
        assert!(c.resolve("Land").is_none());
        assert_eq!(
            c.resolve("GateExpandableFinish").unwrap().template.marker,
            Some(Marker::Finish)
        );
        assert_eq!(
            c.resolve("PlatformTechBase").unwrap().template.shape,
            Shape::Open
        );
        assert_eq!(
            c.resolve("PlatformPlasticBase").unwrap().template.shape,
            Shape::Open
        );
        assert_eq!(
            c.resolve("DecoPlatformBase").unwrap().template.shape,
            Shape::Open
        );
        assert_eq!(
            c.resolve("DecoPlatformSlope2Straight")
                .unwrap()
                .template
                .shape,
            Shape::Straight
        );
        assert_eq!(
            c.resolve("PlatformTechSlope2Start").unwrap().template.shape,
            Shape::Straight
        );
        assert_eq!(c.resolve("RoadTechSlopeStart2x1").unwrap().template.len, 2);
        assert_eq!(
            c.resolve("RoadTechToRoadBump").unwrap().template.shape,
            Shape::Straight
        );
        assert!(c.resolve("RoadTechDiagLeft").is_none());
        assert!(c.resolve("PlatformTechLoopStart").is_none());
        assert_eq!(
            c.resolve("DecoHillSlope2Curve1Out").unwrap().template.shape,
            Shape::Curve
        );
        assert_eq!(
            c.resolve("DecoHillSlope2Curve1Out").unwrap().surface,
            Surface::Dirt
        );
        assert_eq!(
            c.resolve("RoadTechBranchTShaped").unwrap().template.shape,
            Shape::Open
        );
        assert_eq!(
            c.resolve("PlatformTechDiag1").unwrap().template.shape,
            Shape::Open
        );
        assert_eq!(
            c.resolve("WaterGrassRampRoadStraight")
                .unwrap()
                .template
                .shape,
            Shape::Straight
        );
        assert!(c.resolve("DecoHillSlope2StraightX2").is_some());
        assert_eq!(
            c.resolve("RoadTechTiltTransition2UpLeftCurveIn")
                .unwrap()
                .template
                .shape,
            Shape::Curve
        );
    }
}

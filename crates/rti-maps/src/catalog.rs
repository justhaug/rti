//! Block asset catalog: maps vanilla block names to planar geometry
//! templates. The default catalog is embedded (`DEFAULT_CATALOG`); a
//! project can extend or override it with `catalog.toml` in its root.
//!
//! Local frame of a template (direction 0): cells are 32 m; `u` grows to
//! the right, `v` forward. A one-cell straight enters at (16, 0) and exits
//! at (16, 32). Multi-cell footprints extend to +u / +v. The compiler tries
//! the possible rotation/origin conventions and keeps whichever chains the
//! most road, so the catalog only needs to be *internally* consistent.

use regex::Regex;
use rti_core::Surface;
use serde::{Deserialize, Serialize};

pub const CELL: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// `len` cells long, straight through.
    Straight,
    /// Quarter turn spanning `size`×`size` cells, entry bottom of column 0,
    /// exit on the right edge of the last row (a right-hand turn).
    Curve,
    /// `len` cells long, exits shifted `shift` cells to the left (+) or right (-).
    Chicane,
    /// Straight with a race marker (start / finish / checkpoint / multilap).
    Marker,
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
    /// Regex on the block name; named groups `fam` and `shape` are optional.
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
    /// Half width of the drivable surface in metres.
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
    /// Family prefix → surface.
    #[serde(default)]
    pub surfaces: Vec<(String, Surface)>,
}

pub struct Catalog {
    pub templates: Vec<(Regex, Template)>,
    pub surfaces: Vec<(String, Surface)>,
}

/// Resolved geometry for one block name.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub template: Template,
    pub surface: Surface,
    pub family: String,
}

pub const FAMILIES: &str = "RoadTech|RoadDirt|RoadBump|RoadIce|OpenTechRoad|OpenDirtRoad|OpenGrassRoad|OpenIceRoad|SnowRoad|RallyRoad|DesertRoad|PlatformTech|PlatformDirt|PlatformIce|PlatformGrass|PlatformPlastic|RoadWater|RoadTechSpecial";

pub const DEFAULT_CATALOG: &str = r#"
# RTI block catalog (bootstrap). Patterns are regexes on the block name.
# Family prefixes decide the surface; shapes decide geometry.

surfaces = [
  ["RoadTech", "asphalt"], ["RoadBump", "asphalt"], ["RoadDirt", "dirt"], ["RoadIce", "ice"],
  ["OpenTechRoad", "asphalt"], ["OpenDirtRoad", "dirt"], ["OpenGrassRoad", "grass"], ["OpenIceRoad", "ice"],
  ["SnowRoad", "dirt"], ["RallyRoad", "dirt"], ["DesertRoad", "asphalt"],
  ["PlatformTech", "asphalt"], ["PlatformDirt", "dirt"], ["PlatformIce", "ice"], ["PlatformGrass", "grass"],
  ["PlatformPlastic", "asphalt"], ["RoadWater", "asphalt"],
]

[[template]]
pattern = '^(?P<fam>FAM)(Straight|SlopeStraight|TiltStraight|TiltTransition\w*|SlopeUTop|SlopeUBottom|Special\w*|Narrow\w*Straight|TurboStraight|TurboRoulette|Boost\w*|Reset\w*|Fragile\w*|NoBrake\w*|NoSteering\w*|SlowMotion\w*|NoEngine\w*|Cruise\w*|Turbo2?\w*)$'
shape = "straight"

[[template]]
pattern = '^(?P<fam>FAM)(SlopeStart|SlopeEnd|SlopeStartX2|SlopeEndX2|SlopeUp|SlopeDown|SlopeBase)(?P<l>\d)x1$'
shape = "straight"
len = 2
note = "NxM suffix is parsed: len from the first number"

[[template]]
pattern = '^(?P<fam>FAM)(Slope|Tilt)?(Start|End)\d?(Left|Right)?$'
shape = "straight"

[[template]]
pattern = '^(?P<fam>FAM)(Tilt|Slope)?Curve(?P<n>[1-5])(In|Out)?$'
shape = "curve"

[[template]]
pattern = '^(?P<fam>FAM)(Tilt|Slope)?ChicaneX(?P<n>[23])(Tilt)?Left$'
shape = "chicane"
shift = 1

[[template]]
pattern = '^(?P<fam>FAM)(Tilt|Slope)?ChicaneX(?P<n>[23])(Tilt)?Right$'
shape = "chicane"
shift = -1

[[template]]
pattern = '^(?P<fam>FAM)(Slope|Tilt)?Start(Tilt|Slope)?\w*$'
shape = "marker"
marker = "start"

[[template]]
pattern = '^(?P<fam>FAM)(Slope|Tilt)?Finish(Tilt|Slope)?\w*$'
shape = "marker"
marker = "finish"

[[template]]
pattern = '^(?P<fam>FAM)(Slope|Tilt)?StartFinish\w*$'
shape = "marker"
marker = "start_finish"

[[template]]
pattern = '^(?P<fam>FAM)(Slope|Tilt)?Checkpoint(Tilt|Slope)?\w*$'
shape = "marker"
marker = "checkpoint"

[[template]]
pattern = '^(?P<fam>FAM)(Slope|Tilt)?Multilap\w*$'
shape = "marker"
marker = "multilap"

[[template]]
pattern = '^Gate(Expandable)?(Start|Finish|Checkpoint|Multilap)\w*$'
shape = "marker"
marker = "checkpoint"
note = "gates are free-standing markers; the marker kind is refined from the name"
"#;

impl Catalog {
    pub fn parse(toml_text: &str) -> anyhow::Result<Catalog> {
        let file: CatalogFile = toml::from_str(&toml_text.replace("FAM", FAMILIES))?;
        let mut templates = vec![];
        for t in file.template {
            let re = Regex::new(&t.pattern)
                .map_err(|e| anyhow::anyhow!("bad catalog pattern {:?}: {e}", t.pattern))?;
            templates.push((re, t));
        }
        Ok(Catalog {
            templates,
            surfaces: file.surfaces,
        })
    }

    pub fn default_catalog() -> Catalog {
        Self::parse(DEFAULT_CATALOG).expect("embedded catalog is valid")
    }

    /// Default catalog plus optional overrides from `<root>/catalog.toml`
    /// (templates listed there take precedence).
    pub fn load(root: &std::path::Path) -> anyhow::Result<Catalog> {
        let mut cat = Self::default_catalog();
        let p = root.join("catalog.toml");
        if p.exists() {
            let extra = Self::parse(&std::fs::read_to_string(&p)?)?;
            let mut t = extra.templates;
            t.extend(cat.templates);
            cat.templates = t;
            let mut s = extra.surfaces;
            s.extend(cat.surfaces);
            cat.surfaces = s;
        }
        Ok(cat)
    }

    pub fn surface_for(&self, family: &str) -> Surface {
        self.surfaces
            .iter()
            .find(|(f, _)| f == family)
            .map(|(_, s)| *s)
            .unwrap_or(Surface::Asphalt)
    }

    /// Resolve a block name. Marker templates are checked before geometry
    /// templates so that `RoadTechStart` is a marker, not a straight.
    pub fn resolve(&self, name: &str) -> Option<Resolved> {
        let mut best: Option<Resolved> = None;
        for (re, t) in &self.templates {
            if let Some(c) = re.captures(name) {
                let family = c
                    .name("fam")
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| "Gate".into());
                let mut tt = t.clone();
                if let Some(n) = c.name("n").and_then(|m| m.as_str().parse::<u32>().ok()) {
                    match tt.shape {
                        Shape::Curve => tt.size = n,
                        Shape::Chicane => tt.len = n,
                        _ => tt.len = n,
                    }
                }
                if let Some(l) = c.name("l").and_then(|m| m.as_str().parse::<u32>().ok()) {
                    tt.len = l;
                }
                if tt.shape == Shape::Marker {
                    // refine gate markers from the name
                    if family == "Gate" {
                        tt.marker = Some(if name.contains("StartFinish") {
                            Marker::StartFinish
                        } else if name.contains("Start") {
                            Marker::Start
                        } else if name.contains("Finish") {
                            Marker::Finish
                        } else if name.contains("Multilap") {
                            Marker::Multilap
                        } else {
                            Marker::Checkpoint
                        });
                    }
                }
                let surface = tt.surface.unwrap_or_else(|| self.surface_for(&family));
                let r = Resolved {
                    template: tt,
                    surface,
                    family,
                };
                // prefer markers over plain geometry
                let better = match &best {
                    None => true,
                    Some(b) => {
                        b.template.shape != Shape::Marker && r.template.shape == Shape::Marker
                    }
                };
                if better {
                    best = Some(r);
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            c.resolve("GateExpandableFinish").unwrap().template.marker,
            Some(Marker::Finish)
        );
    }
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

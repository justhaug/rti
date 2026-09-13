//! Built-in tracks and a procedural generator. The research loop can add
//! more via `ExperimentSpec::GenerateTrack`.

use rand::{Rng, SeedableRng};
use rti_core::{Surface, Track};
use std::path::Path;

pub fn builtin() -> Vec<Track> {
    use Surface::*;
    let mut v = vec![];
    let mut t = Track::from_segments("straight", 8.0, &[(0.0, 400.0, Asphalt)]);
    t.description = "400 m drag strip. Calibrates acceleration/top speed.".into();
    v.push(t);

    let mut t = Track::from_segments(
        "hairpin",
        8.0,
        &[
            (0.0, 150.0, Asphalt),
            (180.0, 22.0, Asphalt),
            (0.0, 150.0, Asphalt),
        ],
    );
    t.description = "Straight, 180° hairpin, straight. Braking point and apex research.".into();
    v.push(t);

    let mut t = Track::from_segments(
        "s_curves",
        7.0,
        &[
            (0.0, 80.0, Asphalt),
            (60.0, 40.0, Asphalt),
            (-90.0, 35.0, Asphalt),
            (70.0, 30.0, Asphalt),
            (-60.0, 45.0, Asphalt),
            (0.0, 100.0, Asphalt),
        ],
    );
    t.description = "Flowing esses. Line optimisation.".into();
    v.push(t);

    let mut t = Track::from_segments(
        "mixed_surface",
        8.0,
        &[
            (0.0, 100.0, Asphalt),
            (0.0, 80.0, Dirt),
            (90.0, 30.0, Dirt),
            (0.0, 60.0, Grass),
            (-90.0, 30.0, Asphalt),
            (0.0, 60.0, Ice),
            (45.0, 40.0, Ice),
            (0.0, 120.0, Asphalt),
        ],
    );
    t.description = "Asphalt/dirt/grass/ice sections. Surface grip calibration.".into();
    v.push(t);

    let mut t = Track::from_segments(
        "loop",
        8.0,
        &[
            (0.0, 120.0, Asphalt),
            (90.0, 50.0, Asphalt),
            (0.0, 200.0, Asphalt),
            (90.0, 50.0, Asphalt),
            (0.0, 120.0, Asphalt),
            (90.0, 50.0, Asphalt),
            (0.0, 200.0, Asphalt),
            (90.0, 50.0, Asphalt),
        ],
    );
    t.description =
        "Rectangular circuit, one lap. Consistency of line through repeated corners.".into();
    let n = t.nodes.len();
    t.checkpoints = vec![n / 4, n / 2, 3 * n / 4];
    v.push(t);
    v
}

/// Procedural track: random arcs and straights with random surfaces.
pub fn generate(name: &str, seed: u64, segments: usize, half_width: f32) -> Track {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let surfaces = [
        Surface::Asphalt,
        Surface::Asphalt,
        Surface::Asphalt,
        Surface::Dirt,
        Surface::Grass,
        Surface::Ice,
    ];
    let mut segs = vec![(0.0f32, 80.0f32, Surface::Asphalt)];
    for _ in 0..segments {
        let surface = surfaces[rng.random_range(0..surfaces.len())];
        if rng.random_bool(0.4) {
            segs.push((0.0, rng.random_range(40.0..200.0), surface));
        } else {
            let deg: f32 =
                rng.random_range(30.0..150.0) * if rng.random_bool(0.5) { 1.0 } else { -1.0 };
            let radius: f32 = rng.random_range(18.0..70.0);
            segs.push((deg, radius, surface));
        }
    }
    segs.push((0.0, 80.0, Surface::Asphalt));
    let mut t = Track::from_segments(name, half_width, &segs);
    t.description = format!("Procedural track seed={seed} segments={segments}");
    let n = t.nodes.len();
    if n > 12 {
        t.checkpoints = vec![n / 3, 2 * n / 3];
    }
    t
}

/// Load every `*.toml`/`*.json` track in a directory, falling back to the
/// built-ins when the directory is missing or empty.
pub fn load_dir(dir: &Path) -> anyhow::Result<Vec<Track>> {
    let mut out = vec![];
    if dir.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "toml" || ext == "json" {
                out.push(Track::load(&p)?);
            }
        }
    }
    if out.is_empty() {
        out = builtin();
    }
    Ok(out)
}

pub fn write_dir(dir: &Path, tracks: &[Track]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for t in tracks {
        let path = dir.join(format!("{}.toml", t.name));
        std::fs::write(path, toml::to_string_pretty(t)?)?;
    }
    Ok(())
}

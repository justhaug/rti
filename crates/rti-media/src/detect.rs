//! Interestingness: which verified results deserve a video?
//!
//! I = w1·WR improvement + w2·technique novelty + w3·leaderboard significance + w4·visual weirdness

use rti_core::Trajectory;
use rti_research::Session;
use serde::{Deserialize, Serialize};

pub use rti_core::config::InterestWeights;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Interestingness {
    pub wr_improvement: f64,
    pub technique_novelty: f64,
    pub leaderboard_significance: f64,
    pub visual_weirdness: f64,
    pub score: f64,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub track_name: String,
    pub new_trajectory: String,
    pub old_trajectory: Option<String>,
    pub new_time_ms: u32,
    pub old_time_ms: Option<u32>,
    /// Human reference (TMX WR or author time), if the track came from a real map.
    pub reference_ms: Option<u32>,
    pub reference_label: Option<String>,
    pub interest: Interestingness,
    /// Whether the new run is oracle-verified (world starts with "oracle:").
    pub verified: bool,
}

pub fn score(
    new: &Trajectory,
    old: Option<&Trajectory>,
    reference_ms: Option<u32>,
    w: &InterestWeights,
) -> Interestingness {
    let mut i = Interestingness::default();
    // 1. improvement over the previous best (saturates at 500 ms)
    if let Some(o) = old {
        let gain = o.result.time_ms as f64 - new.result.time_ms as f64;
        if gain > 0.0 {
            i.wr_improvement = (gain / 500.0).min(1.0);
            i.reasons
                .push(format!("{gain:.0} ms faster than the previous best"));
        }
    } else {
        i.wr_improvement = 0.3;
        i.reasons.push("first finished run on this track".into());
    }
    // 2. technique novelty: different use of walls / braking / speed profile
    if let Some(o) = old {
        let mut nov: f64 = 0.0;
        let dw = (new.result.wall_hits as f64 - o.result.wall_hits as f64).abs();
        if dw >= 1.0 {
            nov += 0.4;
            i.reasons.push("different wall usage".into());
        }
        let brake = |t: &Trajectory| {
            t.actions.iter().filter(|a| a.brake).count() as f64 / t.actions.len().max(1) as f64
        };
        if (brake(new) - brake(o)).abs() > 0.05 {
            nov += 0.3;
            i.reasons.push("different braking pattern".into());
        }
        if (new.result.max_speed - o.result.max_speed).abs() * 3.6 > 5.0 {
            nov += 0.3;
            i.reasons.push("different top speed".into());
        }
        i.technique_novelty = nov.min(1.0);
    } else {
        i.technique_novelty = 0.2;
    }
    // 3. leaderboard significance: beating a human reference matters most
    if let Some(r) = reference_ms {
        let gain = r as f64 - new.result.time_ms as f64;
        if gain > 0.0 {
            i.leaderboard_significance = (0.5 + gain / 1000.0).min(1.0);
            i.reasons
                .push(format!("{gain:.0} ms under the human reference"));
        } else {
            i.leaderboard_significance = (1.0 - (-gain / 5000.0)).clamp(0.0, 0.4);
        }
    } else {
        i.leaderboard_significance = 0.1;
    }
    // 4. visual weirdness: fast wall contacts, off-track excursions
    let mut weird: f64 = 0.0;
    if new.result.wall_hits > 0 && new.result.finished {
        weird += 0.3;
    }
    if new.result.offtrack_ticks > 20 {
        weird += 0.3;
        i.reasons.push("leaves the road".into());
    }
    if let Some(mx) = new
        .states
        .iter()
        .map(|s| s.yaw_rate.abs())
        .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))))
    {
        if mx > 2.5 {
            weird += 0.4;
            i.reasons.push("violent rotation".into());
        }
    }
    i.visual_weirdness = weird.min(1.0);
    i.score = (i.wr_improvement * w.wr_improvement
        + i.technique_novelty * w.technique_novelty
        + i.leaderboard_significance * w.leaderboard_significance
        + i.visual_weirdness * w.visual_weirdness)
        / (w.wr_improvement
            + w.technique_novelty
            + w.leaderboard_significance
            + w.visual_weirdness)
            .max(1e-9)
        * 2.0;
    i
}

/// Scan the archive for verified bests (or sim bests when `require_verified`
/// is false) that have no media yet and score them.
pub fn scan(
    s: &Session,
    w: &InterestWeights,
    require_verified: bool,
) -> anyhow::Result<Vec<Candidate>> {
    let mut out = vec![];
    let existing: Vec<String> = s
        .archive
        .media(1000)?
        .into_iter()
        .map(|m| m.new_trajectory)
        .collect();
    for t in s.archive.tracks()? {
        let world = if require_verified { "oracle" } else { "sim" };
        let rows = s.archive.best_trajectories(&t.name, world, 3)?;
        let finished: Vec<_> = rows.into_iter().filter(|r| r.finished).collect();
        let Some(best) = finished.first() else {
            continue;
        };
        if existing.contains(&best.hash) {
            continue;
        }
        let Some(new) = s
            .archive
            .trajectory(&rti_core::ContentHash(best.hash.clone()))?
        else {
            continue;
        };
        let old = match finished.get(1) {
            Some(r) => s
                .archive
                .trajectory(&rti_core::ContentHash(r.hash.clone()))?,
            None => None,
        };
        let map = s.archive.map_by_track(&t.name)?;
        let (reference_ms, reference_label) = match &map {
            Some(m) if m.wr_ms.is_some() => (
                m.wr_ms.map(|v| v as u32),
                Some(format!(
                    "TMX WR {}",
                    crate::render::fmt_time(m.wr_ms.unwrap() as u32)
                )),
            ),
            Some(m) if m.author_ms.unwrap_or(0) > 0 => (
                m.author_ms.map(|v| v as u32),
                Some(format!(
                    "author time {}",
                    crate::render::fmt_time(m.author_ms.unwrap() as u32)
                )),
            ),
            _ => (None, None),
        };
        let interest = score(&new, old.as_ref(), reference_ms, w);
        out.push(Candidate {
            track_name: t.name.clone(),
            new_trajectory: best.hash.clone(),
            old_trajectory: finished.get(1).map(|r| r.hash.clone()),
            new_time_ms: new.result.time_ms,
            old_time_ms: old.as_ref().map(|o| o.result.time_ms),
            reference_ms,
            reference_label,
            interest,
            verified: new.world.starts_with("oracle"),
        });
    }
    out.sort_by(|a, b| b.interest.score.partial_cmp(&a.interest.score).unwrap());
    Ok(out)
}

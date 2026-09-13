//! Where did the time come from? Aligns two runs of the same track by
//! centerline progress and reports per-section deltas, exit speeds, wall
//! contacts and the single biggest gain.

use rti_core::{CarState, Trajectory};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Section {
    pub index: usize,
    pub from_m: f32,
    pub to_m: f32,
    /// Time (ms) the new run reached `to_m` minus the old run's time.
    pub cumulative_delta_ms: i32,
    /// Delta gained inside this section alone.
    pub section_delta_ms: i32,
    pub new_exit_speed: f32,
    pub old_exit_speed: f32,
    pub new_wall_hits: u32,
    pub old_wall_hits: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Comparison {
    pub new_time_ms: u32,
    pub old_time_ms: u32,
    pub delta_ms: i32,
    pub sections: Vec<Section>,
    /// Section with the largest gain (most negative section delta).
    pub biggest_gain: Option<Section>,
    pub new_max_speed: f32,
    pub old_max_speed: f32,
    pub new_wall_hits: u32,
    pub old_wall_hits: u32,
    /// Human-readable bullet points for the difference card.
    pub bullets: Vec<String>,
}

/// Tick at which `states` first reach centerline distance `d`.
fn tick_at_progress(states: &[CarState], d: f32) -> Option<u32> {
    states.iter().find(|s| s.progress >= d).map(|s| s.tick)
}

fn speed_at_progress(states: &[CarState], d: f32) -> f32 {
    states
        .iter()
        .find(|s| s.progress >= d)
        .map(|s| s.speed())
        .unwrap_or(0.0)
}

fn walls_by_progress(states: &[CarState], d: f32) -> u32 {
    states
        .iter()
        .find(|s| s.progress >= d)
        .map(|s| s.wall_hits)
        .unwrap_or_else(|| states.last().map(|s| s.wall_hits).unwrap_or(0))
}

pub fn compare(
    new: &Trajectory,
    old: &Trajectory,
    track_len: f32,
    n_sections: usize,
) -> Comparison {
    let n = n_sections.max(2);
    let mut c = Comparison {
        new_time_ms: new.result.time_ms,
        old_time_ms: old.result.time_ms,
        delta_ms: new.result.time_ms as i32 - old.result.time_ms as i32,
        new_max_speed: new.result.max_speed,
        old_max_speed: old.result.max_speed,
        new_wall_hits: new.result.wall_hits,
        old_wall_hits: old.result.wall_hits,
        ..Default::default()
    };
    let mut prev_cum = 0i32;
    let reach = |s: &[CarState]| s.last().map(|x| x.progress).unwrap_or(0.0);
    let common = reach(&new.states)
        .min(reach(&old.states))
        .min(track_len)
        .max(1.0);
    for i in 0..n {
        let from = common * i as f32 / n as f32;
        let to = common * (i + 1) as f32 / n as f32;
        let (Some(tn), Some(to_)) = (
            tick_at_progress(&new.states, to),
            tick_at_progress(&old.states, to),
        ) else {
            break;
        };
        let cum = (tn as i32 - to_ as i32) * rti_core::TICK_MS as i32;
        let sec = Section {
            index: i,
            from_m: from,
            to_m: to,
            cumulative_delta_ms: cum,
            section_delta_ms: cum - prev_cum,
            new_exit_speed: speed_at_progress(&new.states, to),
            old_exit_speed: speed_at_progress(&old.states, to),
            new_wall_hits: walls_by_progress(&new.states, to),
            old_wall_hits: walls_by_progress(&old.states, to),
        };
        prev_cum = cum;
        c.sections.push(sec);
    }
    c.biggest_gain = c
        .sections
        .iter()
        .min_by_key(|s| s.section_delta_ms)
        .filter(|s| s.section_delta_ms < 0)
        .cloned();
    // bullets
    if c.delta_ms != 0 {
        c.bullets.push(format!("{:+} ms overall", c.delta_ms));
    }
    if let Some(g) = &c.biggest_gain {
        c.bullets.push(format!(
            "{} ms gained at {:.0}-{:.0} m",
            -g.section_delta_ms, g.from_m, g.to_m
        ));
        let dv = (g.new_exit_speed - g.old_exit_speed) * 3.6;
        if dv.abs() >= 1.0 {
            c.bullets.push(format!("{:+.1} km/h exit speed there", dv));
        }
        if g.new_wall_hits != g.old_wall_hits {
            c.bullets.push(if g.new_wall_hits > g.old_wall_hits {
                "new wall contact used".into()
            } else {
                "avoids a wall contact".into()
            });
        }
    }
    let dmax = (c.new_max_speed - c.old_max_speed) * 3.6;
    if dmax.abs() >= 2.0 {
        c.bullets.push(format!("{:+.1} km/h top speed", dmax));
    }
    if c.new_wall_hits != c.old_wall_hits && c.bullets.len() < 5 {
        c.bullets.push(format!(
            "{} vs {} wall hits",
            c.new_wall_hits, c.old_wall_hits
        ));
    }
    c
}

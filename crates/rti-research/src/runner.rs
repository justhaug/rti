//! Executes `ExperimentSpec`s. Everything here is deterministic given the
//! archive state and the spec's seed.

use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use rti_core::experiment::SearchMethod;
use rti_core::ledger::CostTimer;
use rti_core::{Action, ContentHash, CostRecord, ExperimentSpec, PhysicsParams, Trajectory};
use rti_nn::{behavior_clone, BcConfig, PolicyBundle};
use rti_oracle::verify;
use rti_search::common::SearchInput;
use rti_search::Budget;
use rti_sim::rollout::to_trajectory;
use rti_sim::{rollout, Sim, TrackGeom};
use serde::{Deserialize, Serialize};

use crate::session::Session;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ExperimentReport {
    pub result: serde_json::Value,
    pub summary: String,
    pub cost: CostRecord,
    pub trajectories: Vec<ContentHash>,
    pub models: Vec<ContentHash>,
    /// Raw ingredients for the value function.
    pub improvement_ms: f64,
    pub improvement_verified: bool,
    pub divergence_before: Option<f32>,
    pub divergence_after: Option<f32>,
    pub novelty: f64,
}

pub fn run_experiment(
    s: &Session,
    spec: &ExperimentSpec,
    experiment_id: &str,
) -> anyhow::Result<ExperimentReport> {
    match spec {
        ExperimentSpec::Search {
            track,
            method,
            budget_ticks,
            seed,
            params,
            warm_start,
            physics,
        } => run_search(
            s,
            track,
            *method,
            *budget_ticks,
            *seed,
            params,
            warm_start.as_ref(),
            physics.as_ref(),
            experiment_id,
        ),
        ExperimentSpec::Verify { trajectory } => run_verify(s, trajectory, experiment_id),
        ExperimentSpec::Calibrate {
            tracks,
            trajectories,
            probe_runs,
            budget_ticks,
            seed,
            physics,
        } => run_calibrate(
            s,
            tracks,
            trajectories,
            *probe_runs,
            *budget_ticks,
            *seed,
            physics.as_ref(),
            experiment_id,
        ),
        ExperimentSpec::TrainBc {
            tracks,
            top_k,
            epochs,
            hidden,
            seed,
            value_head,
        } => run_train_bc(
            s,
            tracks,
            *top_k,
            *epochs,
            *hidden,
            *seed,
            *value_head,
            experiment_id,
        ),
        ExperimentSpec::Benchmark { ticks } => run_benchmark(s, *ticks),
        ExperimentSpec::GenerateTrack {
            name,
            seed,
            segments,
            half_width,
        } => run_generate_track(s, name, *seed, *segments, *half_width),
        ExperimentSpec::ImportMap { source, name } => run_import_map(s, source, name.as_deref()),
        ExperimentSpec::CalibrateReplays {
            tracks,
            generations,
            seed,
        } => run_calibrate_replays(s, tracks, *generations, *seed, experiment_id),
        ExperimentSpec::ImportReplay { source, name } => {
            run_import_replay(s, source, name.as_deref())
        }
    }
}

/// Import a .Replay.Gbx: the embedded map becomes a track (compiled with the
/// catalog; coverage may be partial), the player's inputs become a
/// trajectory in world `human:<login>` with the real race time.
pub fn run_import_replay(
    s: &Session,
    source: &str,
    name: Option<&str>,
) -> anyhow::Result<ExperimentReport> {
    use rti_maps::catalog::Catalog;
    let timer = CostTimer::start();
    let bytes = if let Some(id) = source.strip_prefix("tmx:") {
        rti_maps::TmxClient::new().download_replay(id.trim().parse()?)?
    } else {
        std::fs::read(source)?
    };
    let replay = rti_maps::parse_replay(&bytes)?;
    let ghost = replay
        .ghosts
        .first()
        .ok_or_else(|| anyhow::anyhow!("replay has no ghost"))?;
    anyhow::ensure!(
        !ghost.inputs.is_empty(),
        "replay ghost has no decoded inputs ({:?})",
        ghost.warnings
    );
    let map = replay
        .map
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("replay has no embedded map ({:?})", replay.warnings))?;
    // track: reuse an existing track for this map uid, else compile
    let existing = s
        .archive
        .maps(10_000)?
        .into_iter()
        .find(|m| m.map_uid == map.info.uid && !m.track_name.is_empty());
    let (track, report) = match &existing {
        Some(m) => (s.track(&m.track_name)?, None),
        None => {
            let track_name = match name {
                Some(n) => slugify(n),
                None => slugify(&map.info.name),
            };
            let cat = Catalog::load(&s.root)?;
            let (track, report) = rti_maps::compile_track(map, &cat, &track_name)?;
            rti_maps::catalog::write_track(&s.cfg.tracks_dir, &track)?;
            s.archive.upsert_track(&track)?;
            let gbx_hash = s.archive.cas.put_bytes(&replay.map_bytes)?;
            let maps_dir = std::path::Path::new(&s.cfg.oracle.tm2020_maps_dir);
            if !s.cfg.oracle.tm2020_maps_dir.is_empty() && maps_dir.is_dir() {
                std::fs::create_dir_all(maps_dir.join("RTI"))?;
                std::fs::write(
                    maps_dir.join("RTI").join(format!("{track_name}.Map.Gbx")),
                    &replay.map_bytes,
                )?;
            }
            s.archive.insert_map(&rti_archive::rows::MapRow {
                hash: map.source_hash.clone(),
                tmx_id: None,
                map_uid: map.info.uid.clone(),
                map_name: map.info.name.clone(),
                author: map.info.author.clone(),
                track_name: track_name.clone(),
                author_ms: Some(map.info.author_ms as i64),
                wr_ms: None,
                gbx_hash: Some(gbx_hash.0),
                parsed_hash: None,
                tmx: None,
                report: Some(serde_json::to_value(&report)?),
                created_at: String::new(),
            })?;
            (track, Some(report))
        }
    };
    let replay_hash = s.archive.cas.put_bytes(&bytes)?;
    let actions = rti_maps::replay::inputs_to_actions(&ghost.inputs, ghost.ticks);
    let login = if ghost.login.is_empty() {
        replay.player_login.clone()
    } else {
        ghost.login.clone()
    };
    let traj = Trajectory {
        track_hash: track.hash(),
        track_name: track.name.clone(),
        world: format!("human:{login}"),
        actions,
        states: vec![],
        result: rti_core::RunResult {
            finished: true,
            time_ms: ghost.race_time_ms,
            ticks: ghost.ticks,
            checkpoints_hit: ghost.checkpoint_times_ms.len() as u32,
            progress: track.length(),
            ..Default::default()
        },
        method: "replay".into(),
        parent: None,
    };
    let thash = s.archive.insert_trajectory(&traj, None)?;
    // how does our sim do with the human's inputs? (a free sim/reality probe)
    let (sim, _) = s.sim(&track, None)?;
    let ro = rollout(
        &sim,
        &sim.initial_state(),
        &traj.actions,
        track.max_ticks,
        false,
    );
    let cost = timer.finish(ro.ticks);
    let summary = format!(
        "imported replay of {:?} by {} ({}): {} ms, {} checkpoints, {} input events over {} ticks → trajectory {} on track {}{}; our sim replaying those inputs: {} ({:.0}/{:.0} m, {} ms)",
        replay.map_name,
        replay.player_nickname,
        login,
        ghost.race_time_ms,
        ghost.checkpoint_times_ms.len(),
        ghost.inputs.len(),
        ghost.ticks,
        thash.short(),
        track.name,
        report.as_ref().map(|r| format!(" (compiled: {}/{} blocks chained, finish={})", r.blocks_chained, r.blocks_recognized, r.finish_found)).unwrap_or_default(),
        if ro.result.finished { "finished" } else { "DNF" },
        ro.result.progress,
        track.length(),
        ro.result.time_ms
    );
    Ok(ExperimentReport {
        result: serde_json::json!({
            "track": track.name,
            "trajectory": thash,
            "replay_hash": replay_hash,
            "map_uid": map.info.uid,
            "player": replay.player_nickname,
            "login": login,
            "race_time_ms": ghost.race_time_ms,
            "checkpoint_times_ms": ghost.checkpoint_times_ms,
            "input_events": ghost.inputs.len(),
            "ticks": ghost.ticks,
            "compile": report,
            "sim_replay": ro.result,
            "warnings": replay.warnings,
        }),
        summary,
        cost,
        trajectories: vec![thash],
        models: vec![],
        improvement_ms: 0.0,
        improvement_verified: false,
        divergence_before: None,
        divergence_after: None,
        novelty: 0.5,
    })
}

/// Import every new replay in a directory (e.g. the game's Autosaves).
/// Returns summaries of the imports performed; already-imported files
/// (by content hash) are skipped.
pub fn import_replay_dir(s: &Session, dir: &std::path::Path) -> anyhow::Result<Vec<String>> {
    let mut out = vec![];
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".Replay.Gbx"))
        .collect();
    files.sort();
    for f in files {
        let bytes = std::fs::read(&f)?;
        let h = rti_core::ContentHash::of_bytes(&bytes);
        if s.archive.cas.exists(&h) {
            continue;
        }
        let spec = ExperimentSpec::ImportReplay {
            source: f.display().to_string(),
            name: None,
        };
        let id = s
            .archive
            .begin_experiment(&spec, None, None, &rti_core::Provenance::now())?;
        match run_import_replay(s, &f.display().to_string(), None) {
            Ok(r) => {
                s.archive
                    .finish_experiment(&id, "done", &r.result, &r.cost, None, None, &r.summary)?;
                out.push(r.summary);
            }
            Err(e) => {
                s.archive.finish_experiment(
                    &id,
                    "failed",
                    &serde_json::json!({"error": e.to_string()}),
                    &Default::default(),
                    None,
                    None,
                    &format!("failed: {e}"),
                )?;
                out.push(format!("{}: failed: {e}", f.display()));
            }
        }
    }
    Ok(out)
}

/// Slug for track names derived from map names (strips TM `$xxx` codes).
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_us = true;
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek() {
                Some(&n) if n.is_ascii_hexdigit() => {
                    for _ in 0..3 {
                        if chars.peek().map(|c| c.is_ascii_hexdigit()).unwrap_or(false) {
                            chars.next();
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
            continue;
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "map".into()
    } else {
        out.chars().take(40).collect()
    }
}

/// Download (or read), parse and compile a real map into a track.
pub fn run_import_map(
    s: &Session,
    source: &str,
    name: Option<&str>,
) -> anyhow::Result<ExperimentReport> {
    use rti_maps::catalog::Catalog;
    let timer = CostTimer::start();
    let path = std::path::Path::new(source);
    let (gbx, tmx_meta): (Vec<u8>, Option<rti_maps::TmxMap>) = if path.is_file() {
        (std::fs::read(path)?, None)
    } else {
        let id = rti_maps::TmxClient::parse_id(source)
            .ok_or_else(|| anyhow::anyhow!("{source:?} is neither a file nor a TMX id/URL"))?;
        let client = rti_maps::TmxClient::new();
        let meta = client.map_info(id).ok();
        (client.download(id)?, meta)
    };
    let parsed = rti_maps::parse_map(&gbx)?;
    let track_name = match name {
        Some(n) => slugify(n),
        None => {
            let base = slugify(&parsed.info.name);
            match &tmx_meta {
                Some(m) => format!("tmx{}_{}", m.map_id, base),
                None => base,
            }
        }
    };
    let cat = Catalog::load(&s.root)?;
    let (track, report) = rti_maps::compile_track(&parsed, &cat, &track_name)?;
    let gbx_hash = s.archive.cas.put_bytes(&gbx)?;
    let parsed_hash = s.archive.cas.put_json(&parsed)?;
    // make the map loadable by the game bridge
    let maps_dir = std::path::Path::new(&s.cfg.oracle.tm2020_maps_dir);
    if !s.cfg.oracle.tm2020_maps_dir.is_empty() && maps_dir.is_dir() {
        let dst = maps_dir.join("RTI");
        std::fs::create_dir_all(&dst)?;
        std::fs::write(dst.join(format!("{track_name}.Map.Gbx")), &gbx)?;
    }
    rti_maps::catalog::write_track(&s.cfg.tracks_dir, &track)?;
    let thash = s.archive.upsert_track(&track)?;
    let row = rti_archive::rows::MapRow {
        hash: parsed.source_hash.clone(),
        tmx_id: tmx_meta.as_ref().map(|m| m.map_id as i64),
        map_uid: parsed.info.uid.clone(),
        map_name: parsed.info.name.clone(),
        author: if parsed.info.author_nick.is_empty() {
            parsed.info.author.clone()
        } else {
            parsed.info.author_nick.clone()
        },
        track_name: track_name.clone(),
        author_ms: Some(parsed.info.author_ms as i64),
        wr_ms: tmx_meta.as_ref().and_then(|m| m.wr_ms()).map(|v| v as i64),
        gbx_hash: Some(gbx_hash.0.clone()),
        parsed_hash: Some(parsed_hash.0.clone()),
        tmx: tmx_meta
            .as_ref()
            .map(|m| serde_json::to_value(m).unwrap_or_default()),
        report: Some(serde_json::to_value(&report)?),
        created_at: String::new(),
    };
    s.archive.insert_map(&row)?;
    // human runs from the TMX leaderboard (best few), as warm starts and references
    let mut replays_imported = 0usize;
    if let Some(m) = &tmx_meta {
        if m.replay_count > 0 {
            if let Ok(list) = rti_maps::TmxClient::new().replay_list(m.map_id, 5) {
                for (rid, _t, _who) in list.into_iter().take(3) {
                    match run_import_replay(s, &format!("tmx:{rid}"), Some(&track_name)) {
                        Ok(_) => replays_imported += 1,
                        Err(e) => tracing::warn!(replay = rid, error = %e, "replay import failed"),
                    }
                }
            }
        }
    }
    let cost = timer.finish(0);
    let coverage = if report.blocks_recognized > 0 {
        report.blocks_chained as f64 / report.blocks_recognized as f64
    } else {
        0.0
    };
    let summary = format!(
        "imported {:?} by {} as track {} ({:.0} m, {} nodes, {} cps): {} blocks, {} recognised, {} chained ({:.0}% of road), start={} finish={}, gaps bridged {}; author time {} ms{}; unrecognised top: {}",
        parsed.info.name,
        row.author,
        track_name,
        track.length(),
        track.nodes.len(),
        track.checkpoints.len(),
        report.blocks_total,
        report.blocks_recognized,
        report.blocks_chained,
        coverage * 100.0,
        report.start_found,
        report.finish_found,
        report.bridged_gaps,
        parsed.info.author_ms,
        row.wr_ms.map(|w| format!(", TMX WR {w} ms")).unwrap_or_default(),
        report.unrecognized.iter().take(6).map(|(n, c)| format!("{n}×{c}")).collect::<Vec<_>>().join(", ")
    );
    let summary = if replays_imported > 0 {
        format!("{summary}; imported {replays_imported} TMX replays as human trajectories")
    } else {
        summary
    };
    Ok(ExperimentReport {
        result: serde_json::json!({
            "track": track_name,
            "track_hash": thash,
            "replays_imported": replays_imported,
            "map_hash": parsed.source_hash,
            "gbx_hash": gbx_hash,
            "tmx": row.tmx,
            "report": report,
            "author_ms": parsed.info.author_ms,
            "wr_ms": row.wr_ms,
            "length_m": track.length(),
            "warnings": parsed.warnings,
        }),
        summary,
        cost,
        trajectories: vec![],
        models: vec![],
        improvement_ms: 0.0,
        improvement_verified: false,
        divergence_before: None,
        divergence_after: None,
        novelty: if report.finish_found { 0.8 } else { 0.4 },
    })
}

#[allow(clippy::too_many_arguments)]
fn run_search(
    s: &Session,
    track_name: &str,
    method: SearchMethod,
    budget_ticks: u64,
    seed: u64,
    params: &rti_core::experiment::SearchParams,
    warm_start: Option<&ContentHash>,
    physics: Option<&ContentHash>,
    experiment_id: &str,
) -> anyhow::Result<ExperimentReport> {
    let track = s.track(track_name)?;
    let (sim, phash) = s.sim(&track, physics)?;
    let warm: Option<Trajectory> = match warm_start {
        Some(h) => Some(
            s.archive
                .trajectory(h)?
                .ok_or_else(|| anyhow::anyhow!("warm_start trajectory {h} not found"))?,
        ),
        None => None,
    };
    let policy: Option<PolicyBundle> = match &params.model {
        Some(h) => Some(PolicyBundle::from_bytes(&s.archive.cas.get_bytes(h)?)?),
        None => {
            if matches!(method, SearchMethod::Policy) {
                let m = s.archive.latest_model("policy")?.ok_or_else(|| {
                    anyhow::anyhow!("policy search needs a trained policy (run train_bc first)")
                })?;
                Some(PolicyBundle::from_bytes(
                    &s.archive.cas.get_bytes(&ContentHash(m.hash.clone()))?,
                )?)
            } else {
                None
            }
        }
    };
    let prev_best_sim = s.archive.best_time(track_name, "sim")?;
    let prev_best_oracle = s.archive.best_time(track_name, "oracle")?;
    let timer = CostTimer::start();
    let input = SearchInput {
        sim: &sim,
        params,
        budget: Budget {
            max_ticks: budget_ticks.min(s.cfg.budget.max_ticks_per_experiment),
            max_wall_ms: None,
        },
        seed,
        warm_start: warm.as_ref().map(|t| t.actions.as_slice()),
        policy: policy.as_ref(),
    };
    let out = rti_search::run(method, &input)?;
    let mut cost = timer.finish(out.ticks);
    // materialise the best trajectory with states for later verification
    let ro = rollout(
        &sim,
        &sim.initial_state(),
        &out.best_actions,
        track.max_ticks,
        true,
    );
    cost.sim_ticks += ro.ticks;
    let mut traj = to_trajectory(&sim, out.best_actions.clone(), &ro, method.name(), true);
    traj.parent = warm_start.cloned();
    let thash = s.archive.insert_trajectory(&traj, Some(experiment_id))?;
    let improvement_ms = match (&prev_best_sim, out.best_result.finished) {
        (Some((_, prev)), true) => *prev as f64 - out.best_result.time_ms as f64,
        (None, true) => 0.0,
        _ => 0.0,
    };
    let is_sim_best = prev_best_sim
        .as_ref()
        .map(|(_, p)| out.best_result.time_ms <= *p)
        .unwrap_or(true);
    let beats_verified = out.best_result.finished
        && is_sim_best
        && match &prev_best_oracle {
            Some((_, prev)) => {
                out.best_result.time_ms + s.cfg.research.verify_improvement_ms <= *prev
            }
            None => true,
        };
    // novelty: distance of (mean speed, wall hits, brake fraction) from archived bests
    let novelty = {
        let bests = s.archive.best_trajectories(track_name, "sim", 10)?;
        if bests.len() <= 1 {
            0.5
        } else {
            let mine = out.best_result.mean_speed;
            let d = bests
                .iter()
                .filter(|b| b.hash != thash.0)
                .map(|b| {
                    ((b.time_ms as f32 - out.best_result.time_ms as f32) / 1000.0).abs()
                        + (b.progress as f32 - out.best_result.progress).abs() / 50.0
                })
                .fold(f32::INFINITY, f32::min);
            ((d.min(5.0) / 5.0) as f64 * 0.5 + (mine / 100.0).min(0.5) as f64).clamp(0.0, 1.0)
        }
    };
    let summary = format!(
        "{} on {}: {} {} ms (progress {:.0}/{:.0} m, {} walls) after {} evals / {:.1} Mticks in {:.1}s; prev sim best {:?}, improvement {:+.0} ms{}",
        method.name(),
        track_name,
        if out.best_result.finished { "finished" } else { "DNF" },
        out.best_result.time_ms,
        out.best_result.progress,
        sim.geom.total_len,
        out.best_result.wall_hits,
        out.evaluations,
        out.ticks as f64 / 1e6,
        cost.wall_ms as f64 / 1000.0,
        prev_best_sim.as_ref().map(|p| p.1),
        improvement_ms,
        if beats_verified { " [beats verified best → verify]" } else { "" }
    );
    Ok(ExperimentReport {
        result: serde_json::json!({
            "finished": out.best_result.finished,
            "best_time_ms": out.best_result.time_ms,
            "best_result": out.best_result,
            "evaluations": out.evaluations,
            "ticks": out.ticks,
            "history": out.history,
            "extra": out.extra,
            "trajectory": thash,
            "physics": phash,
            "prev_best_sim_ms": prev_best_sim.map(|p| p.1),
            "prev_best_oracle_ms": prev_best_oracle.map(|p| p.1),
            "improvement_ms": improvement_ms,
            "beats_verified": beats_verified,
            "mticks_per_s": cost.ticks_per_second() / 1e6,
        }),
        summary,
        cost,
        trajectories: vec![thash],
        models: vec![],
        improvement_ms,
        improvement_verified: false,
        divergence_before: None,
        divergence_after: None,
        novelty,
    })
}

fn run_verify(
    s: &Session,
    thash: &ContentHash,
    experiment_id: &str,
) -> anyhow::Result<ExperimentReport> {
    let traj = s
        .archive
        .trajectory(thash)?
        .ok_or_else(|| anyhow::anyhow!("trajectory {thash} not found"))?;
    let track = s.track(&traj.track_name)?;
    let (sim, _) = s.sim(&track, None)?;
    s.oracle.available()?;
    let prev_best_oracle = s.archive.best_time(&traj.track_name, "oracle")?;
    let timer = CostTimer::start();
    let v = verify(s.oracle.as_ref(), &sim, &traj)?;
    let mut cost = timer.finish(traj.actions.len() as u64);
    cost.oracle_ticks = v.oracle_ticks;
    let ohash = s
        .archive
        .insert_trajectory(&v.oracle_trajectory, Some(experiment_id))?;
    s.archive.insert_verification(
        thash,
        &s.oracle.name(),
        Some(&ohash),
        &v.divergence,
        Some(experiment_id),
    )?;
    let stats_before = s.archive.verification_stats()?;
    let improvement_ms = match (&prev_best_oracle, v.oracle_trajectory.result.finished) {
        (Some((_, prev)), true) => *prev as f64 - v.oracle_trajectory.result.time_ms as f64,
        (None, true) => 0.0,
        _ => 0.0,
    };
    let summary = format!(
        "verified {} on {} in {}: sim {} ms vs oracle {} ms ({}); mean pos err {:.2} m, max {:.2} m, first >1 m at tick {:?}; prev verified best {:?}",
        thash.short(),
        traj.track_name,
        s.oracle.name(),
        v.divergence.time_ms_a,
        v.divergence.time_ms_b,
        if v.oracle_trajectory.result.finished { "finished" } else { "DNF in oracle" },
        v.divergence.mean_pos_err,
        v.divergence.max_pos_err,
        v.divergence.first_tick_over_1m,
        prev_best_oracle.as_ref().map(|p| p.1)
    );
    Ok(ExperimentReport {
        result: serde_json::json!({
            "divergence": v.divergence,
            "oracle_trajectory": ohash,
            "oracle_finished": v.oracle_trajectory.result.finished,
            "oracle_time_ms": v.oracle_trajectory.result.time_ms,
            "sim_time_ms": traj.result.time_ms,
            "improvement_ms": improvement_ms,
            "archive_mean_pos_err": stats_before.mean_pos_err,
            "disagreement": v.divergence.mean_pos_err > s.cfg.oracle.disagreement_threshold_m,
        }),
        summary,
        cost,
        trajectories: vec![ohash],
        models: vec![],
        improvement_ms,
        improvement_verified: true,
        divergence_before: None,
        divergence_after: Some(v.divergence.mean_pos_err),
        novelty: 0.0,
    })
}

/// Diverse probe inputs: a centerline-following controller with random
/// gains, speed targets and injected noise so the oracle sees braking,
/// sliding, wall contact and every surface.
fn probe_actions(sim: &Sim, seed: u64, n: usize) -> Vec<Vec<Action>> {
    (0..n)
        .map(|i| {
            let mut rng =
                rand::rngs::StdRng::seed_from_u64(seed.wrapping_mul(1000).wrapping_add(i as u64));
            let k_h: f32 = rng.random_range(1.0..3.0);
            let k_l: f32 = rng.random_range(0.03..0.15);
            let speed_scale: f32 = rng.random_range(0.6..1.3);
            let noise: f32 = rng.random_range(0.0..0.3);
            let look: f32 = rng.random_range(0.15..0.5);
            let acts = std::cell::RefCell::new(Vec::new());
            let rng_cell = std::cell::RefCell::new(rng);
            let max_ticks = sim.geom.track.max_ticks.min(6000);
            rti_sim::rollout_with(
                sim,
                &sim.initial_state(),
                max_ticks,
                max_ticks,
                false,
                |_, st| {
                    let loc = sim.geom.locate(st.x, st.y, st.seg as usize);
                    let v = st.forward_speed().max(1.0);
                    let g = sim.params.surface_grip[loc.surface.index()];
                    let vsafe = sim.geom.safe_speed(
                        st.progress,
                        150.0,
                        sim.params.grip * g * 0.85,
                        sim.params.brake_decel * g * 0.8,
                    ) * speed_scale;
                    let ahead = sim.geom.dir_at(loc.progress + 8.0 + v * look);
                    let herr = rti_sim::geom::wrap_angle(ahead - st.heading);
                    let mut r = rng_cell.borrow_mut();
                    let steer = (k_h * herr - k_l * loc.lateral
                        + noise * r.random_range(-1.0f32..1.0))
                    .clamp(-1.0, 1.0);
                    let a = Action::new(steer, v < vsafe, v > vsafe + 1.0);
                    acts.borrow_mut().push(a);
                    a
                },
            );
            acts.into_inner()
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn run_calibrate(
    s: &Session,
    tracks: &[String],
    trajectories: &[ContentHash],
    probe_runs: usize,
    budget_ticks: u64,
    seed: u64,
    physics: Option<&ContentHash>,
    experiment_id: &str,
) -> anyhow::Result<ExperimentReport> {
    s.oracle.available()?;
    let track_names = if tracks.is_empty() {
        s.track_names()?
    } else {
        tracks.to_vec()
    };
    let (start_hash, start_params) = match physics {
        Some(h) => (
            h.clone(),
            s.archive
                .physics_by_hash(h)?
                .ok_or_else(|| anyhow::anyhow!("unknown physics {h}"))?,
        ),
        None => s.archive.current_physics()?,
    };
    let timer = CostTimer::start();
    // 1. collect oracle telemetry
    struct Probe {
        geom: TrackGeom,
        actions: Vec<Action>,
        oracle_states: Vec<rti_core::CarState>,
    }
    let mut probes: Vec<Probe> = vec![];
    let mut oracle_ticks = 0u64;
    for name in &track_names {
        let track = s.track(name)?;
        let sim = Sim::new(start_params.clone(), TrackGeom::new(track.clone()));
        for acts in probe_actions(&sim, seed, probe_runs.max(1)) {
            let run = s.oracle.run(&track, &acts, track.max_ticks)?;
            oracle_ticks += run.ticks;
            probes.push(Probe {
                geom: TrackGeom::new(track.clone()),
                actions: acts,
                oracle_states: run.states,
            });
        }
    }
    for h in trajectories {
        if let Some(t) = s.archive.trajectory(h)? {
            let track = s.track(&t.track_name)?;
            let run = s.oracle.run(&track, &t.actions, track.max_ticks)?;
            oracle_ticks += run.ticks;
            probes.push(Probe {
                geom: TrackGeom::new(track),
                actions: t.actions,
                oracle_states: run.states,
            });
        }
    }
    anyhow::ensure!(!probes.is_empty(), "no probe data");
    let total_probe_ticks: u64 = probes.iter().map(|p| p.actions.len() as u64).sum();

    // 2. loss = mean position error over all probes (early ticks weighted equally)
    let loss = |params: &PhysicsParams| -> (f64, u64) {
        let per: Vec<(f64, u64)> = probes
            .par_iter()
            .map(|p| {
                let sim = Sim::new(params.clone(), p.geom.clone());
                let ro = rollout(
                    &sim,
                    &sim.initial_state(),
                    &p.actions,
                    p.geom.track.max_ticks,
                    true,
                );
                let n = ro.states.len().min(p.oracle_states.len());
                let mut e = 0.0f64;
                for i in 0..n {
                    let a = ro.states[i];
                    let b = p.oracle_states[i];
                    e += (((a.x - b.x).powi(2) + (a.y - b.y).powi(2)) as f64).sqrt();
                }
                // penalise length mismatch (one world stopped early)
                let missing = p.oracle_states.len().abs_diff(ro.states.len()) as f64;
                (
                    (e + missing * 5.0) / p.oracle_states.len().max(1) as f64,
                    ro.ticks,
                )
            })
            .collect();
        let ticks = per.iter().map(|x| x.1).sum();
        (
            per.iter().map(|x| x.0).sum::<f64>() / per.len() as f64,
            ticks,
        )
    };
    let (before, t0) = loss(&start_params);
    let mut sim_ticks = t0;

    // 3. CEM over log-multipliers of the parameter vector
    let base = start_params.to_vec();
    let dim = base.len();
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed ^ 0xC0FFEE);
    let pop = 32usize;
    let n_elite = 6usize;
    let mut mean = vec![0.0f32; dim];
    let mut std = vec![0.15f32; dim];
    let mut best = (before, start_params.clone());
    let mut gens = 0usize;
    let mut history = vec![];
    while sim_ticks + (pop as u64) * total_probe_ticks
        <= budget_ticks.min(s.cfg.budget.max_ticks_per_experiment)
    {
        let cands: Vec<Vec<f32>> = (0..pop)
            .map(|_| {
                (0..dim)
                    .map(|d| {
                        let z: f32 =
                            rand_distr::Distribution::sample(&rand_distr::StandardNormal, &mut rng);
                        mean[d] + std[d] * z
                    })
                    .collect::<Vec<f32>>()
            })
            .collect();
        let mut scored: Vec<(f64, usize)> = vec![];
        for (i, c) in cands.iter().enumerate() {
            let v: Vec<f32> = base.iter().zip(c).map(|(b, m)| b * m.exp()).collect();
            let p = PhysicsParams::from_vec(&v)?;
            let (l, t) = loss(&p);
            sim_ticks += t;
            scored.push((l, i));
            if l < best.0 {
                best = (l, p);
            }
        }
        scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for d in 0..dim {
            let m: f32 = scored[..n_elite]
                .iter()
                .map(|(_, i)| cands[*i][d])
                .sum::<f32>()
                / n_elite as f32;
            let v: f32 = scored[..n_elite]
                .iter()
                .map(|(_, i)| (cands[*i][d] - m).powi(2))
                .sum::<f32>()
                / n_elite as f32;
            mean[d] = 0.7 * m + 0.3 * mean[d];
            std[d] = (0.7 * v.sqrt() + 0.3 * std[d]).max(0.01);
        }
        gens += 1;
        history.push(serde_json::json!({"gen": gens, "best": best.0, "ticks": sim_ticks}));
    }
    let after = best.0;
    let mut cost = timer.finish(sim_ticks);
    cost.oracle_ticks = oracle_ticks;
    let improved = after < before * 0.98;
    let new_hash = if improved {
        let h = s.archive.insert_physics(
            &best.1,
            &format!("calibrated gen{gens} err={after:.3}"),
            Some(experiment_id),
            Some(after as f32),
        )?;
        s.archive.set_current_physics(&h)?;
        Some(h)
    } else {
        None
    };
    // largest relative parameter changes, for the analyst
    let names = PhysicsParams::names();
    let bv = best.1.to_vec();
    let mut deltas: Vec<(String, f32)> = names
        .iter()
        .zip(base.iter().zip(bv.iter()))
        .map(|(n, (a, b))| (n.to_string(), (b / a).ln()))
        .collect();
    deltas.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
    let summary = format!(
        "calibration on {:?}: {} probes ({} oracle ticks), CEM {} gens / {:.1} Mticks; mean pos err {:.3} → {:.3} m{}; largest changes: {}",
        track_names,
        probes.len(),
        oracle_ticks,
        gens,
        sim_ticks as f64 / 1e6,
        before,
        after,
        if improved { " (adopted as current physics)" } else { " (no significant improvement; unmodelled physics?)" },
        deltas.iter().take(4).map(|(n, d)| format!("{n} {:+.0}%", (d.exp() - 1.0) * 100.0)).collect::<Vec<_>>().join(", ")
    );
    Ok(ExperimentReport {
        result: serde_json::json!({
            "before_mean_pos_err": before,
            "after_mean_pos_err": after,
            "improved": improved,
            "physics_before": start_hash,
            "physics_after": new_hash,
            "generations": gens,
            "probes": probes.len(),
            "history": history,
            "param_log_deltas": deltas,
        }),
        summary,
        cost,
        trajectories: vec![],
        models: vec![],
        improvement_ms: 0.0,
        improvement_verified: false,
        divergence_before: Some(before as f32),
        divergence_after: Some(after as f32),
        novelty: 0.0,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_train_bc(
    s: &Session,
    tracks: &[String],
    top_k: usize,
    epochs: usize,
    hidden: usize,
    seed: u64,
    value_head: bool,
    experiment_id: &str,
) -> anyhow::Result<ExperimentReport> {
    let track_names = if tracks.is_empty() {
        s.track_names()?
    } else {
        tracks.to_vec()
    };
    let timer = CostTimer::start();
    let mut sims: Vec<(Sim, Trajectory)> = vec![];
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    for name in &track_names {
        let track = s.track(name)?;
        let (sim, _) = s.sim(&track, None)?;
        let rows = s.archive.best_trajectories(name, "sim", top_k)?;
        for r in rows.iter().filter(|r| r.finished) {
            if let Some(t) = s.archive.trajectory(&ContentHash(r.hash.clone()))? {
                // DART-style: also add noisy re-executions for state coverage
                for j in 0..3 {
                    let mut acts = t.actions.clone();
                    if j > 0 {
                        for a in acts.iter_mut() {
                            a.steer = (a.steer + rng.random_range(-0.15f32..0.15)).clamp(-1.0, 1.0);
                        }
                    }
                    let ro = rollout(&sim, &sim.initial_state(), &acts, track.max_ticks, false);
                    let tj = to_trajectory(&sim, acts, &ro, "bc_demo", false);
                    sims.push((sim.clone(), tj));
                }
            }
        }
    }
    anyhow::ensure!(
        !sims.is_empty(),
        "no finished trajectories to clone from on {:?}",
        track_names
    );
    let refs: Vec<(&Sim, &Trajectory)> = sims.iter().map(|(a, b)| (a, b)).collect();
    let cfg = BcConfig {
        epochs,
        hidden,
        seed,
        value_head,
        ..Default::default()
    };
    let (bundle, metrics) = behavior_clone(&refs, &cfg)?;
    let sim_ticks: u64 = sims.iter().map(|(_, t)| t.actions.len() as u64).sum();
    // evaluate: deterministic rollout on each track
    let mut evals = vec![];
    let mut eval_ticks = 0u64;
    for name in &track_names {
        let track = s.track(name)?;
        let (sim, _) = s.sim(&track, None)?;
        let (_, ro) = rti_nn::policy_rollout(&sim, &bundle, 0.0, track.max_ticks, 0);
        eval_ticks += ro.ticks;
        evals.push(serde_json::json!({"track": name, "finished": ro.result.finished, "time_ms": ro.result.time_ms, "progress": ro.result.progress}));
    }
    let mut bundle = bundle;
    bundle.meta = serde_json::json!({"tracks": track_names, "top_k": top_k, "epochs": epochs, "hidden": hidden, "seed": seed, "experiment": experiment_id});
    let mhash = s.archive.cas.put_bytes(&bundle.to_bytes())?;
    let metrics_json = serde_json::json!({"n_samples": metrics.n_samples, "final_loss": metrics.final_loss, "initial_loss": metrics.initial_loss, "value_loss": metrics.value_loss, "evals": evals});
    s.archive.insert_model(
        "policy",
        &mhash,
        &bundle.meta,
        &metrics_json,
        Some(experiment_id),
    )?;
    let cost = timer.finish(sim_ticks + eval_ticks);
    let finished = evals
        .iter()
        .filter(|e| e["finished"].as_bool().unwrap_or(false))
        .count();
    let summary = format!(
        "behaviour cloning on {:?}: {} demos / {} samples, loss {:.3} → {:.3}; deterministic policy finishes {}/{} tracks; model {}",
        track_names,
        sims.len(),
        metrics.n_samples,
        metrics.initial_loss,
        metrics.final_loss,
        finished,
        evals.len(),
        mhash.short()
    );
    Ok(ExperimentReport {
        result: serde_json::json!({"model": mhash, "metrics": metrics_json, "demos": sims.len()}),
        summary,
        cost,
        trajectories: vec![],
        models: vec![mhash],
        improvement_ms: 0.0,
        improvement_verified: false,
        divergence_before: None,
        divergence_after: None,
        novelty: 0.3,
    })
}

fn run_benchmark(s: &Session, ticks: u64) -> anyhow::Result<ExperimentReport> {
    let names = s.track_names()?;
    let track = s.track(names.first().ok_or_else(|| anyhow::anyhow!("no tracks"))?)?;
    let (sim, _) = s.sim(&track, None)?;
    let per_run = 3000u32;
    let n = (ticks / per_run as u64).max(32) as usize;
    let seqs: Vec<Vec<Action>> = (0..n)
        .map(|k| {
            (0..per_run)
                .map(|i| {
                    Action::new(
                        ((i + k as u32 * 37) as f32 * 0.013).sin() * 0.5,
                        true,
                        i % 211 < 20,
                    )
                })
                .collect()
        })
        .collect();
    let timer = CostTimer::start();
    let (_, t) = rti_sim::rollout_batch(&sim, &sim.initial_state(), &seqs, per_run);
    let cost = timer.finish(t);
    let mticks = cost.ticks_per_second() / 1e6;
    let summary = format!(
        "sim throughput {:.1} Mticks/s on {} threads ({} ticks in {} ms)",
        mticks,
        rayon::current_num_threads(),
        t,
        cost.wall_ms
    );
    Ok(ExperimentReport {
        result: serde_json::json!({"mticks_per_s": mticks, "threads": rayon::current_num_threads(), "ticks": t}),
        summary,
        cost,
        ..Default::default()
    })
}

fn run_generate_track(
    s: &Session,
    name: &str,
    seed: u64,
    segments: usize,
    half_width: f32,
) -> anyhow::Result<ExperimentReport> {
    anyhow::ensure!(
        s.archive.track_by_name(name)?.is_none(),
        "track {name:?} already exists"
    );
    let track = rti_sim::tracks::generate(name, seed, segments, half_width);
    track.validate()?;
    std::fs::create_dir_all(&s.cfg.tracks_dir)?;
    rti_sim::tracks::write_dir(&s.cfg.tracks_dir, std::slice::from_ref(&track))?;
    let h = s.archive.upsert_track(&track)?;
    let summary = format!(
        "generated track {name} ({:.0} m, {} nodes, {} checkpoints) seed {seed}",
        track.length(),
        track.nodes.len(),
        track.checkpoints.len()
    );
    Ok(ExperimentReport {
        result: serde_json::json!({"track": name, "hash": h, "length_m": track.length(), "nodes": track.nodes.len()}),
        summary,
        novelty: 0.6,
        ..Default::default()
    })
}

/// Fit physics to the archive's human replays (world `human:*`).
pub fn run_calibrate_replays(
    s: &Session,
    tracks: &[String],
    generations: usize,
    seed: u64,
    experiment_id: &str,
) -> anyhow::Result<ExperimentReport> {
    let timer = CostTimer::start();
    let names = if tracks.is_empty() {
        s.track_names()?
    } else {
        tracks.to_vec()
    };
    // cases: (geom, actions, real time, checkpoint count)
    let mut cases: Vec<(TrackGeom, Vec<Action>, u32)> = vec![];
    for name in &names {
        let track = s.track(name)?;
        for row in s.archive.best_trajectories(name, "human", 5)? {
            if let Some(t) = s.archive.trajectory(&ContentHash(row.hash.clone()))? {
                let avg = track.length() / (t.result.time_ms.max(1) as f32 / 1000.0);
                if (8.0..=110.0).contains(&avg) && track.finish.is_some() {
                    let mut acts = t.actions.clone();
                    let hold = acts.last().copied().unwrap_or(Action::full_gas());
                    acts.extend(std::iter::repeat_n(hold, 300));
                    cases.push((TrackGeom::new(track.clone()), acts, t.result.time_ms));
                }
            }
        }
    }
    anyhow::ensure!(!cases.is_empty(), "no usable human replays (need imported replays on tracks that compiled start-to-finish with a plausible average speed)");
    let (start_hash, start_params) = s.archive.current_physics()?;
    let loss = |p: &PhysicsParams| -> (f64, u64, usize) {
        let per: Vec<(f64, u64, bool)> = cases
            .par_iter()
            .map(|(geom, acts, real)| {
                let sim = Sim::new(p.clone(), geom.clone());
                let ro = rollout(&sim, &sim.initial_state(), acts, acts.len() as u32, false);
                if ro.result.finished {
                    (
                        (ro.result.time_ms as f64 - *real as f64).abs(),
                        ro.ticks,
                        true,
                    )
                } else {
                    (
                        20_000.0 + (geom.total_len - ro.result.progress).max(0.0) as f64 * 20.0,
                        ro.ticks,
                        false,
                    )
                }
            })
            .collect();
        (
            per.iter().map(|x| x.0).sum::<f64>() / per.len() as f64,
            per.iter().map(|x| x.1).sum(),
            per.iter().filter(|x| x.2).count(),
        )
    };
    let (before, t0, fin0) = loss(&start_params);
    let mut sim_ticks = t0;
    let base = start_params.to_vec();
    let dim = base.len();
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed ^ 0xB0A7);
    let mut mean = vec![0.0f32; dim];
    let mut std = vec![0.3f32; dim];
    let mut best = (before, start_params.clone(), fin0);
    let pop = 32;
    let n_elite = 6;
    for _gen in 0..generations {
        let cands: Vec<Vec<f32>> = (0..pop)
            .map(|_| {
                (0..dim)
                    .map(|d| {
                        let z: f32 =
                            rand_distr::Distribution::sample(&rand_distr::StandardNormal, &mut rng);
                        mean[d] + std[d] * z
                    })
                    .collect()
            })
            .collect();
        let mut scored: Vec<(f64, usize)> = vec![];
        for (i, c) in cands.iter().enumerate() {
            let v: Vec<f32> = base
                .iter()
                .zip(c)
                .map(|(b, m)| b * m.clamp(-2.5, 2.5).exp())
                .collect();
            let p = PhysicsParams::from_vec(&v)?;
            let (l, t, fin) = loss(&p);
            sim_ticks += t;
            scored.push((l, i));
            if l < best.0 {
                best = (l, p, fin);
            }
        }
        scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for d in 0..dim {
            let m: f32 = scored[..n_elite]
                .iter()
                .map(|(_, i)| cands[*i][d])
                .sum::<f32>()
                / n_elite as f32;
            let v: f32 = scored[..n_elite]
                .iter()
                .map(|(_, i)| (cands[*i][d] - m).powi(2))
                .sum::<f32>()
                / n_elite as f32;
            mean[d] = 0.7 * m + 0.3 * mean[d];
            std[d] = (0.7 * v.sqrt() + 0.3 * std[d]).max(0.01);
        }
    }
    let improved = best.0 < before * 0.98;
    let new_hash = if improved {
        let h = s.archive.insert_physics(
            &best.1,
            &format!("replay-fit {} cases err={:.0} ms", cases.len(), best.0),
            Some(experiment_id),
            None,
        )?;
        s.archive.set_current_physics(&h)?;
        Some(h)
    } else {
        None
    };
    let cost = timer.finish(sim_ticks);
    let summary = format!(
        "replay calibration on {} human runs over {:?}: mean loss {:.0} → {:.0} ms-equivalent, sim finishes {}/{} → {}/{}{}",
        cases.len(),
        names,
        before,
        best.0,
        fin0,
        cases.len(),
        best.2,
        cases.len(),
        if improved { " (adopted as current physics)" } else { " (no significant improvement)" }
    );
    Ok(ExperimentReport {
        result: serde_json::json!({"before": before, "after": best.0, "cases": cases.len(), "finished_before": fin0, "finished_after": best.2, "improved": improved, "physics_before": start_hash, "physics_after": new_hash, "generations": generations}),
        summary,
        cost,
        trajectories: vec![],
        models: vec![],
        improvement_ms: 0.0,
        improvement_verified: false,
        divergence_before: Some(before as f32),
        divergence_after: Some(best.0 as f32),
        novelty: 0.0,
    })
}

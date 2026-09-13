//! Persistent scientific memory for RTI.
//!
//! * DuckDB is the authoritative structured archive (`rti.duckdb`).
//! * A BLAKE3 content-addressed store holds large artifacts.
//! * Every row that represents a result carries provenance and cost.

pub mod cas;
pub mod rows;
pub mod schema;

use anyhow::{bail, Context};
use duckdb::types::ValueRef;
use duckdb::{params, Connection, OptionalExt};
use rti_core::trajectory::Divergence;
use rti_core::{
    ContentHash, CostRecord, ExperimentSpec, PhysicsParams, Provenance, Track, Trajectory,
    ValueScore,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use cas::Cas;
pub use rows::*;

pub struct Archive {
    conn: Connection,
    pub cas: Cas,
    pub root: PathBuf,
    _tmp: Option<tempfile::TempDir>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn json_opt(s: Option<String>) -> Option<Value> {
    s.and_then(|s| serde_json::from_str(&s).ok())
}

fn cost_opt(s: Option<String>) -> Option<CostRecord> {
    s.and_then(|s| serde_json::from_str(&s).ok())
}

impl Archive {
    /// Open (or create) the archive under `data_dir`.
    pub fn open(data_dir: &Path) -> anyhow::Result<Archive> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("creating {}", data_dir.display()))?;
        let db = data_dir.join("rti.duckdb");
        let conn = Connection::open(&db).with_context(|| format!("opening {}", db.display()))?;
        let cas = Cas::open(data_dir.join("cas"))?;
        let a = Archive {
            conn,
            cas,
            root: data_dir.to_path_buf(),
            _tmp: None,
        };
        a.migrate()?;
        Ok(a)
    }

    /// In-memory database with a temporary CAS directory (tests).
    pub fn open_in_memory() -> anyhow::Result<Archive> {
        let tmp = tempfile::tempdir()?;
        let conn = Connection::open_in_memory()?;
        let cas = Cas::open(tmp.path().join("cas"))?;
        let a = Archive {
            conn,
            cas,
            root: tmp.path().to_path_buf(),
            _tmp: Some(tmp),
        };
        a.migrate()?;
        Ok(a)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(schema::DDL)?;
        let v: Option<i64> = self
            .conn
            .query_row("SELECT max(version) FROM schema_version", [], |r| r.get(0))
            .optional()?
            .flatten();
        if v.unwrap_or(0) < schema::SCHEMA_VERSION {
            self.conn.execute(
                "INSERT INTO schema_version VALUES (?, ?)",
                params![schema::SCHEMA_VERSION, now()],
            )?;
        }
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_doc() -> String {
        schema::SCHEMA_DOC.to_string()
    }

    // ------------------------------------------------------------ tracks

    pub fn upsert_track(&self, track: &Track) -> anyhow::Result<ContentHash> {
        let hash = track.hash();
        self.conn.execute(
            "INSERT INTO tracks (hash, name, length_m, n_nodes, json, created_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (hash) DO UPDATE SET name = excluded.name",
            params![hash.as_str(), track.name, track.length() as f64, track.nodes.len() as i64, serde_json::to_string(track)?, now()],
        )?;
        Ok(hash)
    }

    /// Register an imported real-game map (idempotent on `hash`).
    pub fn insert_map(&self, m: &MapRow) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO maps (hash, tmx_id, map_uid, map_name, author, track_name, author_ms, wr_ms, gbx_hash, parsed_hash, tmx_json, report_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (hash) DO UPDATE SET track_name = excluded.track_name, report_json = excluded.report_json, tmx_json = excluded.tmx_json",
            params![
                m.hash,
                m.tmx_id,
                m.map_uid,
                m.map_name,
                m.author,
                m.track_name,
                m.author_ms,
                m.wr_ms,
                m.gbx_hash,
                m.parsed_hash,
                m.tmx.as_ref().map(|v| v.to_string()),
                m.report.as_ref().map(|v| v.to_string()),
                now()
            ],
        )?;
        Ok(())
    }

    /// Resolve a (possibly abbreviated) trajectory hash to the full hash.
    pub fn resolve_trajectory_hash(&self, prefix: &str) -> anyhow::Result<Option<ContentHash>> {
        let p = prefix.trim().trim_end_matches('…');
        if p.len() < 6 {
            return Ok(None);
        }
        let mut st = self.conn.prepare("SELECT hash FROM trajectories WHERE hash LIKE ? || '%' ORDER BY created_at DESC LIMIT 1")?;
        let r: Option<String> = st.query_row(params![p], |r| r.get(0)).ok();
        Ok(r.map(ContentHash))
    }

    /// Resolve a (possibly abbreviated) physics hash to the full hash.
    pub fn resolve_physics_hash(&self, prefix: &str) -> anyhow::Result<Option<ContentHash>> {
        let p = prefix.trim().trim_end_matches('…');
        if p.len() < 6 {
            return Ok(None);
        }
        let mut st = self.conn.prepare(
            "SELECT hash FROM physics WHERE hash LIKE ? || '%' ORDER BY created_at DESC LIMIT 1",
        )?;
        let r: Option<String> = st.query_row(params![p], |r| r.get(0)).ok();
        Ok(r.map(ContentHash))
    }

    pub fn insert_media(&self, m: &MediaRow) -> anyhow::Result<()> {
        let ts = now();
        self.conn.execute(
            "INSERT INTO media (id, kind, track_name, new_trajectory, old_trajectory, score, interest_json, job_json, comparison_json, video_path, video_hash, thumbnail_path, title, description, explanation, tags_json, status, youtube_id, verified, frames, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (id) DO UPDATE SET status = excluded.status, title = excluded.title, description = excluded.description, updated_at = excluded.updated_at",
            params![
                m.id,
                m.kind,
                m.track_name,
                m.new_trajectory,
                m.old_trajectory,
                m.score,
                m.interest.as_ref().map(|v| v.to_string()),
                m.job.as_ref().map(|v| v.to_string()),
                m.comparison.as_ref().map(|v| v.to_string()),
                m.video_path,
                m.video_hash,
                m.thumbnail_path,
                m.title,
                m.description,
                m.explanation,
                serde_json::to_string(&m.tags)?,
                m.status,
                m.youtube_id,
                m.verified,
                m.frames,
                ts,
                ts
            ],
        )?;
        Ok(())
    }

    pub fn update_media(
        &self,
        id: &str,
        status: &str,
        youtube_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE media SET status = ?, youtube_id = COALESCE(?, youtube_id), updated_at = ? WHERE id = ?",
            params![status, youtube_id, now(), id],
        )?;
        Ok(())
    }

    pub fn media(&self, n: usize) -> anyhow::Result<Vec<MediaRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, kind, track_name, new_trajectory, old_trajectory, score, interest_json, job_json, comparison_json, video_path, video_hash, thumbnail_path, title, description, explanation, tags_json, status, youtube_id, verified, frames, created_at, updated_at FROM media ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = st
            .query_map(params![n as i64], |r| {
                let js = |i: usize| -> Result<Option<serde_json::Value>, duckdb::Error> {
                    let s: Option<String> = r.get(i)?;
                    Ok(s.and_then(|t| serde_json::from_str(&t).ok()))
                };
                let tags: Option<String> = r.get(15)?;
                Ok(MediaRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    track_name: r.get(2)?,
                    new_trajectory: r.get(3)?,
                    old_trajectory: r.get(4)?,
                    score: r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                    interest: js(6)?,
                    job: js(7)?,
                    comparison: js(8)?,
                    video_path: r.get(9)?,
                    video_hash: r.get(10)?,
                    thumbnail_path: r.get(11)?,
                    title: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
                    description: r.get::<_, Option<String>>(13)?.unwrap_or_default(),
                    explanation: r.get::<_, Option<String>>(14)?.unwrap_or_default(),
                    tags: tags
                        .and_then(|t| serde_json::from_str(&t).ok())
                        .unwrap_or_default(),
                    status: r.get(16)?,
                    youtube_id: r.get(17)?,
                    verified: r.get::<_, Option<bool>>(18)?.unwrap_or(false),
                    frames: r.get::<_, Option<i64>>(19)?.unwrap_or(0),
                    created_at: r.get(20)?,
                    updated_at: r.get(21)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn media_by_id(&self, id: &str) -> anyhow::Result<Option<MediaRow>> {
        Ok(self.media(100_000)?.into_iter().find(|m| m.id == id))
    }

    pub fn maps(&self, n: usize) -> anyhow::Result<Vec<MapRow>> {
        let mut st = self.conn.prepare(
            "SELECT hash, tmx_id, map_uid, map_name, author, track_name, author_ms, wr_ms, gbx_hash, parsed_hash, tmx_json, report_json, created_at FROM maps ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = st
            .query_map(params![n as i64], |r| {
                let tmx: Option<String> = r.get(10)?;
                let rep: Option<String> = r.get(11)?;
                Ok(MapRow {
                    hash: r.get(0)?,
                    tmx_id: r.get(1)?,
                    map_uid: r.get(2)?,
                    map_name: r.get(3)?,
                    author: r.get(4)?,
                    track_name: r.get(5)?,
                    author_ms: r.get(6)?,
                    wr_ms: r.get(7)?,
                    gbx_hash: r.get(8)?,
                    parsed_hash: r.get(9)?,
                    tmx: tmx.and_then(|t| serde_json::from_str(&t).ok()),
                    report: rep.and_then(|t| serde_json::from_str(&t).ok()),
                    created_at: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn map_by_track(&self, track_name: &str) -> anyhow::Result<Option<MapRow>> {
        Ok(self
            .maps(10_000)?
            .into_iter()
            .find(|m| m.track_name == track_name))
    }

    pub fn tracks(&self) -> anyhow::Result<Vec<TrackRow>> {
        let mut st = self.conn.prepare("SELECT hash, name, length_m, n_nodes, created_at FROM tracks ORDER BY name, created_at")?;
        let rows = st
            .query_map([], |r| {
                Ok(TrackRow {
                    hash: r.get(0)?,
                    name: r.get(1)?,
                    length_m: r.get(2)?,
                    n_nodes: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Latest track registered under `name`.
    pub fn track_by_name(&self, name: &str) -> anyhow::Result<Option<Track>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT json FROM tracks WHERE name = ? ORDER BY created_at DESC LIMIT 1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    pub fn track_by_hash(&self, hash: &ContentHash) -> anyhow::Result<Option<Track>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT json FROM tracks WHERE hash = ?",
                params![hash.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    // ------------------------------------------------------------ physics

    pub fn insert_physics(
        &self,
        params_: &PhysicsParams,
        label: &str,
        source_experiment: Option<&str>,
        oracle_mean_err: Option<f32>,
    ) -> anyhow::Result<ContentHash> {
        let hash = params_.hash();
        self.conn.execute(
            "INSERT INTO physics (hash, label, version, json, source_experiment, oracle_mean_err, is_current, created_at) VALUES (?, ?, ?, ?, ?, ?, FALSE, ?) ON CONFLICT (hash) DO UPDATE SET label = excluded.label, oracle_mean_err = coalesce(excluded.oracle_mean_err, physics.oracle_mean_err)",
            params![
                hash.as_str(),
                label,
                PhysicsParams::VERSION as i64,
                serde_json::to_string(params_)?,
                source_experiment,
                oracle_mean_err.map(|v| v as f64),
                now()
            ],
        )?;
        Ok(hash)
    }

    pub fn set_current_physics(&self, hash: &ContentHash) -> anyhow::Result<()> {
        let exists: i64 = self.conn.query_row(
            "SELECT count(*) FROM physics WHERE hash = ?",
            params![hash.as_str()],
            |r| r.get(0),
        )?;
        if exists == 0 {
            bail!("physics {} not in archive", hash);
        }
        self.conn
            .execute("UPDATE physics SET is_current = FALSE WHERE is_current", [])?;
        self.conn.execute(
            "UPDATE physics SET is_current = TRUE WHERE hash = ?",
            params![hash.as_str()],
        )?;
        Ok(())
    }

    /// The calibrated parameter set in use. Registers `PhysicsParams::default()`
    /// as current if nothing is marked yet.
    pub fn current_physics(&self) -> anyhow::Result<(ContentHash, PhysicsParams)> {
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT hash, json FROM physics WHERE is_current LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((h, j)) = row {
            return Ok((ContentHash(h), serde_json::from_str(&j)?));
        }
        let p = PhysicsParams::default();
        let h = self.insert_physics(&p, "default", None, None)?;
        self.set_current_physics(&h)?;
        Ok((h, p))
    }

    pub fn physics_by_hash(&self, hash: &ContentHash) -> anyhow::Result<Option<PhysicsParams>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT json FROM physics WHERE hash = ?",
                params![hash.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match json {
            Some(j) => Some(serde_json::from_str(&j)?),
            None => None,
        })
    }

    pub fn physics_history(&self, n: usize) -> anyhow::Result<Vec<PhysicsRow>> {
        let mut st = self.conn.prepare(
            "SELECT hash, label, version, source_experiment, oracle_mean_err, is_current, created_at FROM physics ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = st
            .query_map(params![n as i64], |r| {
                Ok(PhysicsRow {
                    hash: r.get(0)?,
                    label: r.get(1)?,
                    version: r.get(2)?,
                    source_experiment: r.get(3)?,
                    oracle_mean_err: r.get(4)?,
                    is_current: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ------------------------------------------------------------ experiments

    pub fn begin_experiment(
        &self,
        spec: &ExperimentSpec,
        hypothesis_id: Option<&str>,
        cycle: Option<i64>,
        provenance: &Provenance,
    ) -> anyhow::Result<String> {
        let id = new_id();
        let (method, track): (Option<String>, Option<String>) = match spec {
            ExperimentSpec::Search { track, method, .. } => {
                (Some(method.name().to_string()), Some(track.clone()))
            }
            ExperimentSpec::Calibrate { tracks, .. } => (None, tracks.first().cloned()),
            ExperimentSpec::ReplayBench { tracks } => {
                (Some("bench".into()), tracks.first().cloned())
            }
            ExperimentSpec::CalibrateReplays { tracks, .. } => {
                (Some("replays".into()), tracks.first().cloned())
            }
            ExperimentSpec::TrainBc { tracks, .. } => (Some("bc".into()), tracks.first().cloned()),
            ExperimentSpec::GenerateTrack { name, .. } => (None, Some(name.clone())),
            ExperimentSpec::ImportMap { name, .. } => (None, name.clone()),
            ExperimentSpec::ImportReplay { name, .. } => (Some("replay".into()), name.clone()),
            ExperimentSpec::Verify { .. } | ExperimentSpec::Benchmark { .. } => (None, None),
        };
        self.conn.execute(
            "INSERT INTO experiments (id, cycle, kind, method, track, spec_json, hypothesis_id, status, provenance_json, started_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'running', ?, ?)",
            params![
                id,
                cycle,
                spec.kind(),
                method,
                track,
                serde_json::to_string(spec)?,
                hypothesis_id,
                serde_json::to_string(provenance)?,
                now()
            ],
        )?;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_experiment(
        &self,
        id: &str,
        status: &str,
        result: &Value,
        cost: &CostRecord,
        value: Option<&ValueScore>,
        value_total: Option<f64>,
        summary: &str,
    ) -> anyhow::Result<()> {
        let n = self.conn.execute(
            "UPDATE experiments SET status = ?, result_json = ?, cost_json = ?, value_json = ?, value_total = ?, summary = ?, finished_at = ? WHERE id = ?",
            params![
                status,
                serde_json::to_string(result)?,
                serde_json::to_string(cost)?,
                value.map(serde_json::to_string).transpose()?,
                value_total,
                summary,
                now(),
                id
            ],
        )?;
        if n == 0 {
            bail!("experiment {id} not found");
        }
        Ok(())
    }

    fn experiment_rows(
        &self,
        sql: &str,
        p: &[&dyn duckdb::ToSql],
    ) -> anyhow::Result<Vec<ExperimentRow>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st
            .query_map(p, |r| {
                let spec: String = r.get(5)?;
                let result: Option<String> = r.get(8)?;
                let cost: Option<String> = r.get(10)?;
                let value: Option<String> = r.get(11)?;
                let prov: String = r.get(13)?;
                Ok(ExperimentRow {
                    id: r.get(0)?,
                    cycle: r.get(1)?,
                    kind: r.get(2)?,
                    method: r.get(3)?,
                    track: r.get(4)?,
                    spec: serde_json::from_str(&spec).unwrap_or(Value::Null),
                    hypothesis_id: r.get(6)?,
                    status: r.get(7)?,
                    result: json_opt(result),
                    summary: r.get(9)?,
                    cost: cost_opt(cost),
                    value: json_opt(value),
                    value_total: r.get(12)?,
                    provenance: serde_json::from_str(&prov).unwrap_or(Value::Null),
                    started_at: r.get(14)?,
                    finished_at: r.get(15)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    const EXP_COLS: &'static str = "id, cycle, kind, method, track, spec_json, hypothesis_id, status, result_json, summary, cost_json, value_json, value_total, provenance_json, started_at, finished_at";

    pub fn recent_experiments(&self, n: usize) -> anyhow::Result<Vec<ExperimentRow>> {
        let sql = format!(
            "SELECT {} FROM experiments ORDER BY started_at DESC LIMIT ?",
            Self::EXP_COLS
        );
        self.experiment_rows(&sql, &[&(n as i64)])
    }

    pub fn experiment(&self, id: &str) -> anyhow::Result<Option<ExperimentRow>> {
        let sql = format!("SELECT {} FROM experiments WHERE id = ?", Self::EXP_COLS);
        Ok(self.experiment_rows(&sql, &[&id])?.into_iter().next())
    }

    pub fn experiments_by_cycle(&self, cycle: i64) -> anyhow::Result<Vec<ExperimentRow>> {
        let sql = format!(
            "SELECT {} FROM experiments WHERE cycle = ? ORDER BY started_at",
            Self::EXP_COLS
        );
        self.experiment_rows(&sql, &[&cycle])
    }

    pub fn experiment_count(&self) -> anyhow::Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM experiments", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    // ------------------------------------------------------------ trajectories

    /// Store the full trajectory (inputs + optional telemetry) in the CAS and
    /// a summary row. Idempotent on content hash.
    pub fn insert_trajectory(
        &self,
        traj: &Trajectory,
        experiment_id: Option<&str>,
    ) -> anyhow::Result<ContentHash> {
        let hash = self.cas.put_json(traj)?;
        self.conn.execute(
            "INSERT INTO trajectories (hash, track_name, track_hash, world, method, experiment_id, finished, time_ms, progress, ticks, has_states, parent, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (hash) DO NOTHING",
            params![
                hash.as_str(),
                traj.track_name,
                traj.track_hash.as_str(),
                traj.world,
                if traj.method.is_empty() { None } else { Some(traj.method.as_str()) },
                experiment_id,
                traj.result.finished,
                traj.result.time_ms as i64,
                traj.result.progress as f64,
                traj.result.ticks as i64,
                !traj.states.is_empty(),
                traj.parent.as_ref().map(|p| p.as_str().to_string()),
                now()
            ],
        )?;
        Ok(hash)
    }

    pub fn trajectory(&self, hash: &ContentHash) -> anyhow::Result<Option<Trajectory>> {
        if !self.cas.exists(hash) {
            return Ok(None);
        }
        Ok(Some(self.cas.get_json(hash)?))
    }

    fn trajectory_rows(
        &self,
        sql: &str,
        p: &[&dyn duckdb::ToSql],
    ) -> anyhow::Result<Vec<TrajectoryRow>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st
            .query_map(p, |r| {
                let time_ms: i64 = r.get(7)?;
                let ticks: i64 = r.get(9)?;
                Ok(TrajectoryRow {
                    hash: r.get(0)?,
                    track_name: r.get(1)?,
                    track_hash: r.get(2)?,
                    world: r.get(3)?,
                    method: r.get(4)?,
                    experiment_id: r.get(5)?,
                    finished: r.get(6)?,
                    time_ms: time_ms as u32,
                    progress: r.get(8)?,
                    ticks: ticks as u64,
                    has_states: r.get(10)?,
                    parent: r.get(11)?,
                    created_at: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    const TRAJ_COLS: &'static str =
        "hash, track_name, track_hash, world, method, experiment_id, finished, time_ms, progress, ticks, has_states, parent, created_at";

    /// Best trajectories for a track: finished runs by time, then unfinished
    /// by progress. `world_prefix` ("sim", "oracle", "" for any) filters the
    /// world column by prefix.
    pub fn best_trajectories(
        &self,
        track_name: &str,
        world_prefix: &str,
        n: usize,
    ) -> anyhow::Result<Vec<TrajectoryRow>> {
        let sql = format!(
            "SELECT {} FROM trajectories WHERE track_name = ? AND world LIKE ? ORDER BY finished DESC, CASE WHEN finished THEN time_ms ELSE 0 END ASC, progress DESC, created_at ASC LIMIT ?",
            Self::TRAJ_COLS
        );
        let like = format!("{world_prefix}%");
        self.trajectory_rows(&sql, &[&track_name, &like, &(n as i64)])
    }

    pub fn best_time(
        &self,
        track_name: &str,
        world_prefix: &str,
    ) -> anyhow::Result<Option<(ContentHash, u32)>> {
        let like = format!("{world_prefix}%");
        let row: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT hash, time_ms FROM trajectories WHERE track_name = ? AND world LIKE ? AND finished ORDER BY time_ms ASC, created_at ASC LIMIT 1",
                params![track_name, like],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(h, t)| (ContentHash(h), t as u32)))
    }

    pub fn trajectories_for_experiment(&self, id: &str) -> anyhow::Result<Vec<TrajectoryRow>> {
        let sql = format!(
            "SELECT {} FROM trajectories WHERE experiment_id = ? ORDER BY created_at",
            Self::TRAJ_COLS
        );
        self.trajectory_rows(&sql, &[&id])
    }

    pub fn trajectory_row(&self, hash: &ContentHash) -> anyhow::Result<Option<TrajectoryRow>> {
        let sql = format!(
            "SELECT {} FROM trajectories WHERE hash = ?",
            Self::TRAJ_COLS
        );
        Ok(self
            .trajectory_rows(&sql, &[&hash.as_str()])?
            .into_iter()
            .next())
    }

    pub fn trajectory_count(&self) -> anyhow::Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM trajectories", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    // ------------------------------------------------------------ verifications

    pub fn insert_verification(
        &self,
        trajectory_hash: &ContentHash,
        oracle: &str,
        oracle_trajectory_hash: Option<&ContentHash>,
        div: &Divergence,
        experiment_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let id = new_id();
        self.conn.execute(
            "INSERT INTO verifications (id, trajectory_hash, oracle, oracle_trajectory_hash, experiment_id, divergence_json, mean_pos_err, max_pos_err, time_ms_sim, time_ms_oracle, both_finished, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                trajectory_hash.as_str(),
                oracle,
                oracle_trajectory_hash.map(|h| h.as_str().to_string()),
                experiment_id,
                serde_json::to_string(div)?,
                div.mean_pos_err as f64,
                div.max_pos_err as f64,
                div.time_ms_a as i64,
                div.time_ms_b as i64,
                div.both_finished,
                now()
            ],
        )?;
        Ok(id)
    }

    pub fn recent_verifications(&self, n: usize) -> anyhow::Result<Vec<VerificationRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, trajectory_hash, oracle, oracle_trajectory_hash, experiment_id, divergence_json, mean_pos_err, max_pos_err, time_ms_sim, time_ms_oracle, both_finished, created_at FROM verifications ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = st
            .query_map(params![n as i64], |r| {
                let div: String = r.get(5)?;
                let a: i64 = r.get(8)?;
                let b: i64 = r.get(9)?;
                Ok(VerificationRow {
                    id: r.get(0)?,
                    trajectory_hash: r.get(1)?,
                    oracle: r.get(2)?,
                    oracle_trajectory_hash: r.get(3)?,
                    experiment_id: r.get(4)?,
                    divergence: serde_json::from_str(&div).unwrap_or(Value::Null),
                    mean_pos_err: r.get(6)?,
                    max_pos_err: r.get(7)?,
                    time_ms_sim: a as u32,
                    time_ms_oracle: b as u32,
                    both_finished: r.get(10)?,
                    created_at: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn verification_stats(&self) -> anyhow::Result<VerificationStats> {
        let row: (i64, Option<f64>, Option<f64>) = self.conn.query_row(
            "SELECT count(*), avg(mean_pos_err), avg(abs(time_ms_sim - time_ms_oracle)) FROM verifications",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(VerificationStats {
            n: row.0 as u64,
            mean_pos_err: row.1.unwrap_or(0.0),
            mean_abs_time_diff_ms: row.2.unwrap_or(0.0),
        })
    }

    // ------------------------------------------------------------ models

    pub fn insert_model(
        &self,
        kind: &str,
        hash: &ContentHash,
        meta: &Value,
        metrics: &Value,
        experiment_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO models (hash, kind, experiment_id, meta_json, metrics_json, created_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (hash) DO UPDATE SET metrics_json = excluded.metrics_json",
            params![hash.as_str(), kind, experiment_id, serde_json::to_string(meta)?, serde_json::to_string(metrics)?, now()],
        )?;
        Ok(())
    }

    pub fn models(&self, kind: &str, n: usize) -> anyhow::Result<Vec<ModelRow>> {
        let mut st = self.conn.prepare(
            "SELECT hash, kind, experiment_id, meta_json, metrics_json, created_at FROM models WHERE kind = ? ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = st
            .query_map(params![kind, n as i64], |r| {
                let meta: String = r.get(3)?;
                let metrics: String = r.get(4)?;
                Ok(ModelRow {
                    hash: r.get(0)?,
                    kind: r.get(1)?,
                    experiment_id: r.get(2)?,
                    meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
                    metrics: serde_json::from_str(&metrics).unwrap_or(Value::Null),
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn latest_model(&self, kind: &str) -> anyhow::Result<Option<ModelRow>> {
        Ok(self.models(kind, 1)?.into_iter().next())
    }

    // ------------------------------------------------------------ findings

    pub fn insert_finding(&self, f: &Finding) -> anyhow::Result<String> {
        let id = if f.id.is_empty() {
            new_id()
        } else {
            f.id.clone()
        };
        self.conn.execute(
            "INSERT INTO findings (id, cycle, kind, title, body, confidence, experiment_ids_json, tags_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                f.cycle,
                f.kind,
                f.title,
                f.body,
                f.confidence as f64,
                serde_json::to_string(&f.experiment_ids)?,
                serde_json::to_string(&f.tags)?,
                now()
            ],
        )?;
        Ok(id)
    }

    fn finding_rows(&self, sql: &str, p: &[&dyn duckdb::ToSql]) -> anyhow::Result<Vec<Finding>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st
            .query_map(p, |r| {
                let conf: f64 = r.get(5)?;
                let ids: String = r.get(6)?;
                let tags: String = r.get(7)?;
                Ok(Finding {
                    id: r.get(0)?,
                    cycle: r.get(1)?,
                    kind: r.get(2)?,
                    title: r.get(3)?,
                    body: r.get(4)?,
                    confidence: conf as f32,
                    experiment_ids: serde_json::from_str(&ids).unwrap_or_default(),
                    tags: serde_json::from_str(&tags).unwrap_or_default(),
                    created_at: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    const FINDING_COLS: &'static str =
        "id, cycle, kind, title, body, confidence, experiment_ids_json, tags_json, created_at";

    pub fn findings(&self, n: usize) -> anyhow::Result<Vec<Finding>> {
        let sql = format!(
            "SELECT {} FROM findings ORDER BY created_at DESC LIMIT ?",
            Self::FINDING_COLS
        );
        self.finding_rows(&sql, &[&(n as i64)])
    }

    pub fn findings_by_kind(&self, kind: &str, n: usize) -> anyhow::Result<Vec<Finding>> {
        let sql = format!(
            "SELECT {} FROM findings WHERE kind = ? ORDER BY created_at DESC LIMIT ?",
            Self::FINDING_COLS
        );
        self.finding_rows(&sql, &[&kind, &(n as i64)])
    }

    pub fn finding(&self, id: &str) -> anyhow::Result<Option<Finding>> {
        let sql = format!("SELECT {} FROM findings WHERE id = ?", Self::FINDING_COLS);
        Ok(self.finding_rows(&sql, &[&id])?.into_iter().next())
    }

    // ------------------------------------------------------------ hypotheses

    pub fn insert_hypothesis(
        &self,
        text: &str,
        rationale: &str,
        cycle: Option<i64>,
    ) -> anyhow::Result<String> {
        let id = new_id();
        let t = now();
        self.conn.execute(
            "INSERT INTO hypotheses (id, cycle, text, rationale, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'open', ?, ?)",
            params![id, cycle, text, rationale, t, t],
        )?;
        Ok(id)
    }

    pub fn set_hypothesis_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        let n = self.conn.execute(
            "UPDATE hypotheses SET status = ?, updated_at = ? WHERE id = ?",
            params![status, now(), id],
        )?;
        if n == 0 {
            bail!("hypothesis {id} not found");
        }
        Ok(())
    }

    pub fn hypotheses(&self, status: Option<&str>, n: usize) -> anyhow::Result<Vec<HypothesisRow>> {
        let (sql, p): (String, Vec<Box<dyn duckdb::ToSql>>) = match status {
            Some(s) => (
                "SELECT id, cycle, text, rationale, status, created_at, updated_at FROM hypotheses WHERE status = ? ORDER BY created_at DESC LIMIT ?".into(),
                vec![Box::new(s.to_string()), Box::new(n as i64)],
            ),
            None => (
                "SELECT id, cycle, text, rationale, status, created_at, updated_at FROM hypotheses ORDER BY created_at DESC LIMIT ?".into(),
                vec![Box::new(n as i64)],
            ),
        };
        let refs: Vec<&dyn duckdb::ToSql> = p.iter().map(|b| b.as_ref()).collect();
        let mut st = self.conn.prepare(&sql)?;
        let rows = st
            .query_map(refs.as_slice(), |r| {
                Ok(HypothesisRow {
                    id: r.get(0)?,
                    cycle: r.get(1)?,
                    text: r.get(2)?,
                    rationale: r.get(3)?,
                    status: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ------------------------------------------------------------ ledger

    pub fn ledger_add(
        &self,
        category: &str,
        ref_id: &str,
        cost: &CostRecord,
        note: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO ledger (id, category, ref_id, sim_ticks, oracle_ticks, wall_ms, cpu_ms, llm_calls, llm_prompt_tokens, llm_completion_tokens, llm_usd, note, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                new_id(),
                category,
                ref_id,
                cost.sim_ticks as i64,
                cost.oracle_ticks as i64,
                cost.wall_ms as i64,
                cost.cpu_ms as i64,
                cost.llm_calls as i64,
                cost.llm_prompt_tokens as i64,
                cost.llm_completion_tokens as i64,
                cost.llm_usd,
                note,
                now()
            ],
        )?;
        Ok(())
    }

    fn ledger_sum(
        &self,
        where_clause: &str,
        p: &[&dyn duckdb::ToSql],
    ) -> anyhow::Result<CostRecord> {
        let sql = format!(
            "SELECT coalesce(sum(sim_ticks),0), coalesce(sum(oracle_ticks),0), coalesce(sum(wall_ms),0), coalesce(sum(cpu_ms),0), coalesce(sum(llm_calls),0), coalesce(sum(llm_prompt_tokens),0), coalesce(sum(llm_completion_tokens),0), coalesce(sum(llm_usd),0.0) FROM ledger {where_clause}"
        );
        let row = self.conn.query_row(&sql, p, |r| {
            Ok(CostRecord {
                sim_ticks: r.get::<_, i128>(0)? as u64,
                oracle_ticks: r.get::<_, i128>(1)? as u64,
                wall_ms: r.get::<_, i128>(2)? as u64,
                cpu_ms: r.get::<_, i128>(3)? as u64,
                llm_calls: r.get::<_, i128>(4)? as u32,
                llm_prompt_tokens: r.get::<_, i128>(5)? as u64,
                llm_completion_tokens: r.get::<_, i128>(6)? as u64,
                llm_usd: r.get(7)?,
            })
        })?;
        Ok(row)
    }

    pub fn ledger_totals(&self) -> anyhow::Result<CostRecord> {
        self.ledger_sum("", &[])
    }

    pub fn ledger_by_category(&self) -> anyhow::Result<Vec<(String, CostRecord)>> {
        let mut st = self
            .conn
            .prepare("SELECT DISTINCT category FROM ledger ORDER BY category")?;
        let cats: Vec<String> = st
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = vec![];
        for c in cats {
            let cost = self.ledger_sum("WHERE category = ?", &[&c])?;
            out.push((c, cost));
        }
        Ok(out)
    }

    pub fn llm_spend_total(&self) -> anyhow::Result<f64> {
        let v: f64 =
            self.conn
                .query_row("SELECT coalesce(sum(usd), 0.0) FROM llm_calls", [], |r| {
                    r.get(0)
                })?;
        Ok(v)
    }

    // ------------------------------------------------------------ llm calls

    #[allow(clippy::too_many_arguments)]
    pub fn insert_llm_call(
        &self,
        role: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        usd: f64,
        latency_ms: u64,
        cycle: Option<i64>,
        experiment_id: Option<&str>,
        ok: bool,
        error: Option<&str>,
    ) -> anyhow::Result<String> {
        let id = new_id();
        self.conn.execute(
            "INSERT INTO llm_calls (id, role, model, prompt_tokens, completion_tokens, usd, latency_ms, cycle, experiment_id, ok, error, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                role,
                model,
                prompt_tokens as i64,
                completion_tokens as i64,
                usd,
                latency_ms as i64,
                cycle,
                experiment_id,
                ok,
                error,
                now()
            ],
        )?;
        Ok(id)
    }

    pub fn llm_call_stats(&self) -> anyhow::Result<Vec<LlmModelStats>> {
        let mut st = self.conn.prepare(
            "SELECT model, count(*), coalesce(sum(prompt_tokens),0), coalesce(sum(completion_tokens),0), coalesce(sum(usd),0.0) FROM llm_calls GROUP BY model ORDER BY sum(usd) DESC",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok(LlmModelStats {
                    model: r.get(0)?,
                    n: r.get::<_, i64>(1)? as u64,
                    prompt_tokens: r.get::<_, i128>(2)? as u64,
                    completion_tokens: r.get::<_, i128>(3)? as u64,
                    usd: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn recent_llm_calls(&self, n: usize) -> anyhow::Result<Vec<LlmCallRow>> {
        let mut st = self.conn.prepare(
            "SELECT id, role, model, prompt_tokens, completion_tokens, usd, latency_ms, cycle, experiment_id, ok, error, created_at FROM llm_calls ORDER BY created_at DESC LIMIT ?",
        )?;
        let rows = st
            .query_map(params![n as i64], |r| {
                Ok(LlmCallRow {
                    id: r.get(0)?,
                    role: r.get(1)?,
                    model: r.get(2)?,
                    prompt_tokens: r.get::<_, i64>(3)? as u64,
                    completion_tokens: r.get::<_, i64>(4)? as u64,
                    usd: r.get(5)?,
                    latency_ms: r.get::<_, i64>(6)? as u64,
                    cycle: r.get(7)?,
                    experiment_id: r.get(8)?,
                    ok: r.get(9)?,
                    error: r.get(10)?,
                    created_at: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ------------------------------------------------------------ tasks

    pub fn insert_task(&self, t: &TaskRow) -> anyhow::Result<String> {
        let id = if t.id.is_empty() {
            new_id()
        } else {
            t.id.clone()
        };
        let ts = now();
        self.conn.execute(
            "INSERT INTO tasks (id, cycle, kind, title, description, status, priority, branch, result_json, cost_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                t.cycle,
                t.kind,
                t.title,
                t.description,
                if t.status.is_empty() { "pending" } else { t.status.as_str() },
                t.priority,
                t.branch,
                t.result_json.as_ref().map(serde_json::to_string).transpose()?,
                t.cost.as_ref().map(serde_json::to_string).transpose()?,
                ts,
                ts
            ],
        )?;
        Ok(id)
    }

    pub fn update_task(
        &self,
        id: &str,
        status: &str,
        result: Option<&Value>,
        cost: Option<&CostRecord>,
        branch: Option<&str>,
    ) -> anyhow::Result<()> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = ?, result_json = coalesce(?, result_json), cost_json = coalesce(?, cost_json), branch = coalesce(?, branch), updated_at = ? WHERE id = ?",
            params![
                status,
                result.map(serde_json::to_string).transpose()?,
                cost.map(serde_json::to_string).transpose()?,
                branch,
                now(),
                id
            ],
        )?;
        if n == 0 {
            bail!("task {id} not found");
        }
        Ok(())
    }

    fn task_rows(&self, sql: &str, p: &[&dyn duckdb::ToSql]) -> anyhow::Result<Vec<TaskRow>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st
            .query_map(p, |r| {
                let result: Option<String> = r.get(8)?;
                let cost: Option<String> = r.get(9)?;
                Ok(TaskRow {
                    id: r.get(0)?,
                    cycle: r.get(1)?,
                    kind: r.get(2)?,
                    title: r.get(3)?,
                    description: r.get(4)?,
                    status: r.get(5)?,
                    priority: r.get(6)?,
                    branch: r.get(7)?,
                    result_json: json_opt(result),
                    cost: cost_opt(cost),
                    created_at: r.get(10)?,
                    updated_at: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    const TASK_COLS: &'static str =
        "id, cycle, kind, title, description, status, priority, branch, result_json, cost_json, created_at, updated_at";

    pub fn pending_tasks(&self) -> anyhow::Result<Vec<TaskRow>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE status = 'pending' ORDER BY priority DESC, created_at ASC",
            Self::TASK_COLS
        );
        self.task_rows(&sql, &[])
    }

    pub fn tasks(&self, n: usize) -> anyhow::Result<Vec<TaskRow>> {
        let sql = format!(
            "SELECT {} FROM tasks ORDER BY created_at DESC LIMIT ?",
            Self::TASK_COLS
        );
        self.task_rows(&sql, &[&(n as i64)])
    }

    pub fn task(&self, id: &str) -> anyhow::Result<Option<TaskRow>> {
        let sql = format!("SELECT {} FROM tasks WHERE id = ?", Self::TASK_COLS);
        Ok(self.task_rows(&sql, &[&id])?.into_iter().next())
    }

    // ------------------------------------------------------------ cycles

    pub fn begin_cycle(&self, brain: &str, model: &str) -> anyhow::Result<i64> {
        let max: Option<i64> = self
            .conn
            .query_row("SELECT max(id) FROM cycles", [], |r| r.get(0))?;
        let id = max.unwrap_or(0) + 1;
        self.conn.execute(
            "INSERT INTO cycles (id, started_at, brain, model) VALUES (?, ?, ?, ?)",
            params![id, now(), brain, model],
        )?;
        Ok(id)
    }

    pub fn finish_cycle(&self, id: i64, summary: &str, cost: &CostRecord) -> anyhow::Result<()> {
        let n = self.conn.execute(
            "UPDATE cycles SET finished_at = ?, summary = ?, cost_json = ? WHERE id = ?",
            params![now(), summary, serde_json::to_string(cost)?, id],
        )?;
        if n == 0 {
            bail!("cycle {id} not found");
        }
        Ok(())
    }

    fn cycle_rows(&self, sql: &str, p: &[&dyn duckdb::ToSql]) -> anyhow::Result<Vec<CycleRow>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st
            .query_map(p, |r| {
                let cost: Option<String> = r.get(4)?;
                Ok(CycleRow {
                    id: r.get(0)?,
                    started_at: r.get(1)?,
                    finished_at: r.get(2)?,
                    summary: r.get(3)?,
                    cost: cost_opt(cost),
                    brain: r.get(5)?,
                    model: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn last_cycle(&self) -> anyhow::Result<Option<CycleRow>> {
        Ok(self
            .cycle_rows("SELECT id, started_at, finished_at, summary, cost_json, brain, model FROM cycles ORDER BY id DESC LIMIT 1", &[])?
            .into_iter()
            .next())
    }

    pub fn recent_cycles(&self, n: usize) -> anyhow::Result<Vec<CycleRow>> {
        self.cycle_rows(
            "SELECT id, started_at, finished_at, summary, cost_json, brain, model FROM cycles ORDER BY id DESC LIMIT ?",
            &[&(n as i64)],
        )
    }

    pub fn cycle_count(&self) -> anyhow::Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM cycles", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    // ------------------------------------------------------------ analytics

    /// Cost-effectiveness per (method, track) over search experiments. Uses
    /// `result_json.best_time_ms` / `result_json.finished` and `cost_json`.
    pub fn method_stats(&self) -> anyhow::Result<Vec<MethodStats>> {
        let mut st = self.conn.prepare(
            "SELECT coalesce(method,'?'), coalesce(track,'?'), result_json, cost_json FROM experiments WHERE kind = 'search' AND status = 'done'",
        )?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut agg: BTreeMap<(String, String), MethodStats> = BTreeMap::new();
        for (method, track, result, cost) in rows {
            let e = agg
                .entry((method.clone(), track.clone()))
                .or_insert_with(|| MethodStats {
                    method,
                    track,
                    n_runs: 0,
                    finished_runs: 0,
                    best_time_ms: None,
                    sim_ticks: 0,
                    wall_ms: 0,
                    llm_usd: 0.0,
                });
            e.n_runs += 1;
            if let Some(res) = json_opt(result) {
                let finished = res
                    .get("finished")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if finished {
                    e.finished_runs += 1;
                    if let Some(t) = res.get("best_time_ms").and_then(|v| v.as_u64()) {
                        e.best_time_ms = Some(match e.best_time_ms {
                            Some(b) => b.min(t as u32),
                            None => t as u32,
                        });
                    }
                }
            }
            if let Some(c) = cost_opt(cost) {
                e.sim_ticks += c.sim_ticks;
                e.wall_ms += c.wall_ms;
                e.llm_usd += c.llm_usd;
            }
        }
        Ok(agg.into_values().collect())
    }

    /// Generic read-only query returning one JSON object per row. Used by the
    /// CLI and the LLM `query_archive` tool.
    pub fn query_json(&self, sql: &str) -> anyhow::Result<Vec<Value>> {
        let head = sql.trim_start().to_ascii_lowercase();
        if !(head.starts_with("select") || head.starts_with("with")) {
            bail!("only SELECT/WITH queries are allowed");
        }
        let mut st = self.conn.prepare(sql)?;
        let mut rows = st.query([])?;
        let mut out = vec![];
        let mut names: Option<Vec<String>> = None;
        while let Some(row) = rows.next()? {
            let names = names.get_or_insert_with(|| row.as_ref().column_names());
            let mut obj = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                let v = row.get_ref(i)?;
                obj.insert(name.clone(), value_ref_to_json(v));
            }
            out.push(Value::Object(obj));
        }
        Ok(out)
    }

    /// One-line health summary for the CLI.
    pub fn status(&self) -> anyhow::Result<Value> {
        let count = |t: &str| -> anyhow::Result<i64> {
            Ok(self
                .conn
                .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))?)
        };
        let (cas_n, cas_bytes) = self.cas.stats()?;
        Ok(serde_json::json!({
            "tracks": count("tracks")?,
            "physics": count("physics")?,
            "experiments": count("experiments")?,
            "trajectories": count("trajectories")?,
            "verifications": count("verifications")?,
            "models": count("models")?,
            "findings": count("findings")?,
            "hypotheses": count("hypotheses")?,
            "tasks": count("tasks")?,
            "cycles": count("cycles")?,
            "llm_calls": count("llm_calls")?,
            "cas_objects": cas_n,
            "cas_bytes": cas_bytes,
            "ledger": self.ledger_totals()?,
        }))
    }
}

fn value_ref_to_json(v: ValueRef<'_>) -> Value {
    use serde_json::json;
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Boolean(b) => json!(b),
        ValueRef::TinyInt(i) => json!(i),
        ValueRef::SmallInt(i) => json!(i),
        ValueRef::Int(i) => json!(i),
        ValueRef::BigInt(i) => json!(i),
        ValueRef::HugeInt(i) => {
            if let Ok(x) = i64::try_from(i) {
                json!(x)
            } else {
                json!(i as f64)
            }
        }
        ValueRef::UHugeInt(i) => {
            if let Ok(x) = u64::try_from(i) {
                json!(x)
            } else {
                json!(i as f64)
            }
        }
        ValueRef::UTinyInt(i) => json!(i),
        ValueRef::USmallInt(i) => json!(i),
        ValueRef::UInt(i) => json!(i),
        ValueRef::UBigInt(i) => json!(i),
        ValueRef::Float(f) => json!(f),
        ValueRef::Double(f) => json!(f),
        ValueRef::Decimal(d) => {
            let s = d.to_string();
            s.parse::<f64>().map(|f| json!(f)).unwrap_or(json!(s))
        }
        ValueRef::Timestamp(unit, t) => {
            use duckdb::types::TimeUnit;
            let micros = match unit {
                TimeUnit::Second => t * 1_000_000,
                TimeUnit::Millisecond => t * 1_000,
                TimeUnit::Microsecond => t,
                TimeUnit::Nanosecond => t / 1_000,
            };
            match chrono::DateTime::<chrono::Utc>::from_timestamp_micros(micros) {
                Some(dt) => json!(dt.to_rfc3339()),
                None => json!(t),
            }
        }
        ValueRef::Text(b) => json!(String::from_utf8_lossy(b)),
        ValueRef::Blob(b) => json!(format!("<blob {} bytes>", b.len())),
        ValueRef::Date32(d) => {
            match chrono::DateTime::<chrono::Utc>::from_timestamp(d as i64 * 86_400, 0) {
                Some(dt) => json!(dt.format("%Y-%m-%d").to_string()),
                None => json!(d),
            }
        }
        other => json!(format!("{other:?}")),
    }
}

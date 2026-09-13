use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rti_archive::Archive;
use rti_core::{PhysicsParams, RtiConfig, Track};
use rti_llm::{LlmClient, Usage, UsageSink};
use rti_oracle::Oracle;
use rti_sim::{Sim, TrackGeom};

/// Buffered LLM-usage records; flushed into the archive by the cycle.
#[derive(Default)]
pub struct UsageBuffer {
    pub records: Mutex<Vec<(String, String, Usage, u64, bool, Option<String>)>>,
}

/// Newtype so the buffer can be shared with the LLM client (orphan rules).
pub struct SharedUsage(pub std::sync::Arc<UsageBuffer>);

impl UsageSink for SharedUsage {
    fn record(
        &self,
        role: &str,
        model: &str,
        usage: &Usage,
        latency_ms: u64,
        ok: bool,
        error: Option<&str>,
    ) {
        self.0.records.lock().unwrap().push((
            role.into(),
            model.into(),
            usage.clone(),
            latency_ms,
            ok,
            error.map(|s| s.to_string()),
        ));
    }
}

/// Everything a cycle needs: config, archive, oracle, LLM client.
pub struct Session {
    pub cfg: RtiConfig,
    pub root: PathBuf,
    pub archive: Archive,
    pub oracle: Box<dyn Oracle>,
    pub llm: LlmClient,
    pub usage: std::sync::Arc<UsageBuffer>,
}

impl Session {
    pub fn open(root: &Path) -> anyhow::Result<Session> {
        let cfg = RtiConfig::load(root)?;
        Self::with_config(root, cfg)
    }

    pub fn with_config(root: &Path, cfg: RtiConfig) -> anyhow::Result<Session> {
        let archive = Archive::open(&cfg.data_dir)?;
        let oracle = rti_oracle::from_config(&cfg.oracle)?;
        let usage = std::sync::Arc::new(UsageBuffer::default());
        let llm = LlmClient::new(&cfg.llm).with_sink(Box::new(SharedUsage(usage.clone())));
        if cfg.budget.threads > 0 {
            let _ = rayon::ThreadPoolBuilder::new()
                .num_threads(cfg.budget.threads)
                .build_global();
        }
        let s = Session {
            cfg,
            root: root.to_path_buf(),
            archive,
            oracle,
            llm,
            usage,
        };
        s.sync_tracks()?;
        Ok(s)
    }

    /// Register tracks from the tracks dir (or built-ins) into the archive.
    pub fn sync_tracks(&self) -> anyhow::Result<usize> {
        let tracks = rti_sim::tracks::load_dir(&self.cfg.tracks_dir)?;
        if !self.cfg.tracks_dir.is_dir() {
            rti_sim::tracks::write_dir(&self.cfg.tracks_dir, &tracks)?;
        }
        for t in &tracks {
            self.archive.upsert_track(t)?;
        }
        Ok(tracks.len())
    }

    pub fn track(&self, name: &str) -> anyhow::Result<Track> {
        self.archive
            .track_by_name(name)?
            .ok_or_else(|| anyhow::anyhow!("unknown track {name:?}"))
    }

    pub fn track_names(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.archive.tracks()?.into_iter().map(|t| t.name).collect())
    }

    /// Simulator for a track with the current (or a specific) physics.
    pub fn sim(
        &self,
        track: &Track,
        physics: Option<&rti_core::ContentHash>,
    ) -> anyhow::Result<(Sim, rti_core::ContentHash)> {
        let (hash, params): (rti_core::ContentHash, PhysicsParams) = match physics {
            Some(h) => (
                h.clone(),
                self.archive
                    .physics_by_hash(h)?
                    .ok_or_else(|| anyhow::anyhow!("unknown physics {h}"))?,
            ),
            None => self.archive.current_physics()?,
        };
        Ok((Sim::new(params, TrackGeom::new(track.clone())), hash))
    }

    /// Flush buffered LLM usage into the archive ledger.
    pub fn flush_usage(
        &self,
        cycle: Option<i64>,
        experiment_id: Option<&str>,
    ) -> anyhow::Result<rti_core::CostRecord> {
        let recs: Vec<_> = std::mem::take(&mut *self.usage.records.lock().unwrap());
        let mut cost = rti_core::CostRecord::default();
        for (role, model, usage, latency, ok, err) in recs {
            self.archive.insert_llm_call(
                &role,
                &model,
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.usd,
                latency,
                cycle,
                experiment_id,
                ok,
                err.as_deref(),
            )?;
            cost.llm_calls += 1;
            cost.llm_prompt_tokens += usage.prompt_tokens;
            cost.llm_completion_tokens += usage.completion_tokens;
            cost.llm_usd += usage.usd;
        }
        Ok(cost)
    }

    pub fn repo_docs(&self) -> String {
        let mut s = String::new();
        for f in ["prompts/coder.md", "AGENTS.md"] {
            if let Ok(t) = std::fs::read_to_string(self.root.join(f)) {
                s.push_str(&t);
                s.push_str("\n\n");
            }
        }
        s
    }

    pub fn prompt(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join("prompts").join(format!("{name}.md")))
            .unwrap_or_else(|_| default_prompt(name))
    }
}

fn default_prompt(name: &str) -> String {
    match name {
        "researcher" => "You are the RTI researcher. Propose one experiment as JSON with keys hypothesis, rationale, prediction, experiment, value_estimate, coding_task.".into(),
        "critic" => "You are the RTI critic. Reply with JSON {verdict, score, critique, revised_experiment}.".into(),
        "analyst" => "You are the RTI analyst. Reply with JSON {title, kind, body, confidence, hypothesis_status, information_gain, follow_ups, tags}.".into(),
        _ => String::new(),
    }
}

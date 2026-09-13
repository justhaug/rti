use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rti_archive::rows::Finding;
use rti_core::{ExperimentSpec, Provenance};
use rti_llm::Message;
use rti_research::{ExperimentReport, Session};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub seq: u64,
    pub ts: String,
    pub kind: String,
    pub text: String,
}

#[derive(Default)]
pub struct LoopControl {
    pub running: AtomicBool,
    pub stop_requested: AtomicBool,
    pub cycles_done: AtomicU64,
    pub cycles_target: AtomicU64,
    pub last_report: Mutex<Option<serde_json::Value>>,
    pub last_error: Mutex<Option<String>>,
}

#[derive(Default)]
pub struct ChatState {
    pub history: Vec<Message>,
    pub busy: bool,
}

pub struct AppState {
    pub session: Arc<Mutex<Session>>,
    pub loop_ctl: LoopControl,
    pub chat: Mutex<ChatState>,
    pub events: Mutex<VecDeque<Event>>,
    pub event_seq: AtomicU64,
    pub directives: Mutex<Vec<String>>,
}

impl AppState {
    pub fn open(root: &Path) -> anyhow::Result<AppState> {
        let session = Session::open(root)?;
        let mut directives = vec![];
        if let Ok(t) = std::fs::read_to_string(session.cfg.data_dir.join("directives.md")) {
            directives.extend(
                t.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.to_string()),
            );
        }
        Ok(AppState {
            session: Arc::new(Mutex::new(session)),
            loop_ctl: LoopControl::default(),
            chat: Mutex::new(ChatState::default()),
            events: Mutex::new(VecDeque::new()),
            event_seq: AtomicU64::new(0),
            directives: Mutex::new(directives),
        })
    }

    pub fn push_event(&self, kind: &str, text: &str) {
        let seq = self.event_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut ev = self.events.lock().unwrap();
        ev.push_back(Event {
            seq,
            ts: chrono::Utc::now().to_rfc3339(),
            kind: kind.into(),
            text: text.into(),
        });
        while ev.len() > 500 {
            ev.pop_front();
        }
        tracing::info!(kind, "{text}");
    }

    pub fn events_since(&self, since: u64) -> Vec<Event> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.seq > since)
            .cloned()
            .collect()
    }

    pub fn loop_status(&self) -> serde_json::Value {
        serde_json::json!({
            "running": self.loop_ctl.running.load(Ordering::SeqCst),
            "stop_requested": self.loop_ctl.stop_requested.load(Ordering::SeqCst),
            "cycles_done": self.loop_ctl.cycles_done.load(Ordering::SeqCst),
            "cycles_target": self.loop_ctl.cycles_target.load(Ordering::SeqCst),
            "last_report": *self.loop_ctl.last_report.lock().unwrap(),
            "last_error": *self.loop_ctl.last_error.lock().unwrap(),
        })
    }

    /// Record a user directive: persisted to data/directives.md and inserted
    /// as a finding of kind "directive" so the researcher's context sees it.
    pub fn add_directive(&self, text: &str) -> anyhow::Result<()> {
        self.directives.lock().unwrap().push(text.to_string());
        let s = self.session.lock().unwrap();
        let path = s.cfg.data_dir.join("directives.md");
        let mut all = std::fs::read_to_string(&path).unwrap_or_default();
        all.push_str(text.trim());
        all.push('\n');
        std::fs::write(&path, all)?;
        s.archive.insert_finding(&Finding {
            kind: "directive".into(),
            title: format!(
                "User directive: {}",
                rti_research::context::truncate(text, 60)
            ),
            body: text.to_string(),
            confidence: 1.0,
            tags: vec!["directive".into()],
            ..Default::default()
        })?;
        self.push_event("directive", text);
        Ok(())
    }

    /// One research cycle followed by draining the task queue. Holds the
    /// session lock for the duration.
    pub fn cycle_once(self: &Arc<Self>) -> anyhow::Result<serde_json::Value> {
        let s = self.session.lock().unwrap();
        let r = match rti_research::run_cycle(&s) {
            Ok(r) => r,
            Err(e) => {
                *self.loop_ctl.last_error.lock().unwrap() = Some(e.to_string());
                self.push_event("error", &format!("cycle failed: {e:#}"));
                return Err(e);
            }
        };
        self.loop_ctl.cycles_done.fetch_add(1, Ordering::SeqCst);
        self.push_event(
            "cycle",
            &format!(
                "cycle {} [{}] V={:.2} ${:.4}: {} — {}",
                r.cycle,
                r.brain,
                r.value_total,
                r.cost.llm_usd,
                r.proposal.hypothesis,
                r.report.summary
            ),
        );
        for f in &r.follow_ups {
            self.push_event("follow-up", f);
        }
        loop {
            match rti_research::tasks::run_next_task(&s) {
                Ok(Some((id, msg))) => self.push_event("task", &format!("{id}: {msg}")),
                Ok(None) => break,
                Err(e) => {
                    self.push_event("error", &format!("task failed: {e:#}"));
                    break;
                }
            }
        }
        let v = serde_json::json!({
            "cycle": r.cycle, "brain": r.brain, "experiment_id": r.experiment_id,
            "hypothesis": r.proposal.hypothesis, "summary": r.report.summary,
            "title": r.conclusion.title, "value_total": r.value_total, "cost": r.cost, "follow_ups": r.follow_ups,
        });
        *self.loop_ctl.last_report.lock().unwrap() = Some(v.clone());
        Ok(v)
    }

    /// Start the background loop (no-op if already running). `cycles` = 0 → until stopped/budget.
    pub fn loop_start(self: &Arc<Self>, cycles: u64) -> bool {
        if self.loop_ctl.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.loop_ctl.stop_requested.store(false, Ordering::SeqCst);
        self.loop_ctl.cycles_target.store(cycles, Ordering::SeqCst);
        let start = self.loop_ctl.cycles_done.load(Ordering::SeqCst);
        let me = self.clone();
        std::thread::Builder::new()
            .name("rti-loop".into())
            .spawn(move || {
                me.push_event(
                    "loop",
                    &format!(
                        "research loop started (cycles={})",
                        if cycles == 0 {
                            "∞".to_string()
                        } else {
                            cycles.to_string()
                        }
                    ),
                );
                loop {
                    if me.loop_ctl.stop_requested.load(Ordering::SeqCst) {
                        break;
                    }
                    let done = me.loop_ctl.cycles_done.load(Ordering::SeqCst) - start;
                    if cycles != 0 && done >= cycles {
                        break;
                    }
                    if let Err(e) = me.cycle_once() {
                        if e.to_string().contains("budget") {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                me.loop_ctl.running.store(false, Ordering::SeqCst);
                me.push_event("loop", "research loop stopped");
            })
            .expect("spawn loop thread");
        true
    }

    pub fn loop_stop(&self) {
        self.loop_ctl.stop_requested.store(true, Ordering::SeqCst);
    }

    /// Run an experiment spec synchronously (same bookkeeping as the CLI).
    pub fn run_spec(&self, mut spec: ExperimentSpec) -> anyhow::Result<(String, ExperimentReport)> {
        let s = self.session.lock().unwrap();
        rti_research::cycle::validate_spec(&s, &mut spec)?;
        let id = s
            .archive
            .begin_experiment(&spec, None, None, &Provenance::now())?;
        match rti_research::run_experiment(&s, &spec, &id) {
            Ok(r) => {
                s.archive
                    .finish_experiment(&id, "done", &r.result, &r.cost, None, None, &r.summary)?;
                s.archive.ledger_add(spec.kind(), &id, &r.cost, "server")?;
                self.push_event("experiment", &format!("{id}: {}", r.summary));
                Ok((id, r))
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
                self.push_event("error", &format!("experiment {id} failed: {e:#}"));
                Err(e)
            }
        }
    }
}

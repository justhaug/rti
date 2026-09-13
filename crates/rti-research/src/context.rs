//! Summarises the archive into the "accumulated knowledge" the brain reads.

use std::fmt::Write;

use crate::session::Session;

pub fn build_context(s: &Session) -> anyhow::Result<String> {
    let cfg = &s.cfg.research;
    let mut out = String::new();
    let a = &s.archive;
    let totals = a.ledger_totals()?;
    let llm_usd = a.llm_spend_total()?;
    let (phash, physics) = a.current_physics()?;
    let vstats = a.verification_stats()?;
    let _ = writeln!(out, "# RTI archive summary\n");
    let _ = writeln!(
        out,
        "cycles: {}  experiments: {}  trajectories: {}  findings: {}  oracle: {}  sim throughput: ~{:.0} Mticks/s",
        a.cycle_count()?,
        a.experiment_count()?,
        a.trajectory_count()?,
        a.findings(1000)?.len(),
        s.oracle.name(),
        totals.ticks_per_second().max(1.0) / 1e6
    );
    let _ = writeln!(
        out,
        "compute so far: {:.1} Mticks sim, {} oracle ticks, {:.1} min wall, LLM ${:.3} of ${:.2} lifetime budget (${:.2}/cycle cap)",
        totals.mticks(),
        totals.oracle_ticks,
        totals.wall_ms as f64 / 60000.0,
        llm_usd,
        s.cfg.budget.max_total_usd,
        s.cfg.budget.max_usd_per_cycle
    );
    let _ = writeln!(
        out,
        "(hashes may be abbreviated to a unique prefix in specs)"
    );
    let _ = writeln!(out, "current physics: {} (version {}); sim/oracle verifications: {} (mean pos err {:.2} m, mean |Δt| {:.0} ms); disagreement threshold {:.1} m\n", phash.short(), physics.hash().short(), vstats.n, vstats.mean_pos_err, vstats.mean_abs_time_diff_ms, s.cfg.oracle.disagreement_threshold_m);

    let _ = writeln!(out, "## Tracks (best sim time / best verified time)");
    for t in a.tracks()? {
        let bs = a.best_time(&t.name, "sim")?;
        let bo = a.best_time(&t.name, "oracle")?;
        let n = a.best_trajectories(&t.name, "", 1000)?.len();
        let _ = writeln!(
            out,
            "- {} ({:.0} m, {} nodes): sim best {} | verified best {} | {} trajectories",
            t.name,
            t.length_m,
            t.n_nodes,
            bs.map(|(h, ms)| format!("{} ms [{}]", ms, h.short()))
                .unwrap_or_else(|| "none".into()),
            bo.map(|(h, ms)| format!("{} ms [{}]", ms, h.short()))
                .unwrap_or_else(|| "none".into()),
            n
        );
    }

    let _ = writeln!(out, "\n## Method efficiency (search experiments)");
    let stats = a.method_stats()?;
    if stats.is_empty() {
        let _ = writeln!(out, "(none yet)");
    }
    for m in stats {
        let _ = writeln!(
            out,
            "- {} on {}: {} runs ({} finished), best {} ms, {:.1} Mticks, {:.0} s wall, ${:.3}",
            m.method,
            m.track,
            m.n_runs,
            m.finished_runs,
            m.best_time_ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".into()),
            m.sim_ticks as f64 / 1e6,
            m.wall_ms as f64 / 1000.0,
            m.llm_usd
        );
    }

    let _ = writeln!(out, "\n## Models");
    let models = a.models("policy", 5)?;
    if models.is_empty() {
        let _ = writeln!(out, "(none yet)");
    }
    for m in models {
        let _ = writeln!(
            out,
            "- policy {} ({}): metrics {}",
            short(&m.hash),
            m.created_at,
            truncate(&m.metrics.to_string(), 300)
        );
    }

    let _ = writeln!(out, "\n## Recent experiments (newest first)");
    for e in a.recent_experiments(cfg.context_experiments)? {
        let _ = writeln!(
            out,
            "- [{}] cycle {} {} {}{}: {} | value {:.2} | {}",
            e.id,
            e.cycle.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
            e.kind,
            e.method.as_deref().unwrap_or(""),
            e.track
                .as_ref()
                .map(|t| format!(" on {t}"))
                .unwrap_or_default(),
            e.status,
            e.value_total.unwrap_or(0.0),
            truncate(e.summary.as_deref().unwrap_or(""), 260)
        );
    }

    let _ = writeln!(out, "\n## Recent verifications");
    let vs = a.recent_verifications(8)?;
    if vs.is_empty() {
        let _ = writeln!(out, "(none yet)");
    }
    for v in vs {
        let _ = writeln!(
            out,
            "- traj {} in {}: mean err {:.2} m, max {:.2} m, sim {} ms vs oracle {} ms{}",
            short(&v.trajectory_hash),
            v.oracle,
            v.mean_pos_err,
            v.max_pos_err,
            v.time_ms_sim,
            v.time_ms_oracle,
            if v.both_finished {
                ""
            } else {
                " (not both finished)"
            }
        );
    }

    let _ = writeln!(out, "\n## Findings (newest first)");
    let fs = a.findings(cfg.context_findings)?;
    if fs.is_empty() {
        let _ = writeln!(out, "(none yet)");
    }
    for f in fs {
        let _ = writeln!(
            out,
            "- [{}] ({}, conf {:.2}) {}: {}",
            f.id,
            f.kind,
            f.confidence,
            f.title,
            truncate(&f.body.replace('\n', " "), 400)
        );
    }

    let _ = writeln!(out, "\n## Open hypotheses");
    let hs = a.hypotheses(Some("open"), 10)?;
    if hs.is_empty() {
        let _ = writeln!(out, "(none)");
    }
    for h in hs {
        let _ = writeln!(out, "- [{}] {}", h.id, truncate(&h.text, 200));
    }

    let _ = writeln!(out, "\n## Pending tasks");
    let ts = a.pending_tasks()?;
    if ts.is_empty() {
        let _ = writeln!(out, "(none)");
    }
    for t in ts {
        let _ = writeln!(
            out,
            "- [{}] {} (prio {}): {}",
            t.id, t.kind, t.priority, t.title
        );
    }
    Ok(out)
}

pub fn short(h: &str) -> &str {
    &h[..12.min(h.len())]
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

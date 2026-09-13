//! End-to-end: the scripted brain runs several cycles against the hidden-sim
//! oracle in a temporary project, exercising runner, archive, tasks and value.
use rti_core::RtiConfig;
use rti_research::{run_cycle, Session};

#[test]
fn scripted_loop_runs_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RtiConfig::default();
    cfg.research.brain = "scripted".into();
    cfg.budget.default_ticks = 2_000_000;
    cfg.data_dir = dir.path().join("data");
    cfg.tracks_dir = dir.path().join("tracks");
    let s = Session::with_config(dir.path(), cfg).unwrap();
    assert_eq!(s.archive.tracks().unwrap().len(), 5);
    let mut kinds = vec![];
    for _ in 0..4 {
        let r = run_cycle(&s).unwrap();
        kinds.push(r.proposal.experiment.kind().to_string());
        assert!(r.value_total.is_finite());
        while let Some((_, msg)) = rti_research::tasks::run_next_task(&s).unwrap() {
            assert!(!msg.starts_with("failed"), "{msg}");
        }
    }
    assert_eq!(kinds[0], "benchmark");
    assert!(kinds.iter().any(|k| k == "search"));
    assert_eq!(s.archive.cycle_count().unwrap(), 4);
    assert!(s.archive.experiment_count().unwrap() >= 4);
    assert!(s.archive.findings(10).unwrap().len() >= 4);
    let totals = s.archive.ledger_totals().unwrap();
    assert!(totals.sim_ticks > 1_000_000);
    let ctx = rti_research::context::build_context(&s).unwrap();
    assert!(ctx.contains("## Tracks") && ctx.contains("hairpin"));
    // a search that finished should have queued and run a verification
    assert!(s.archive.verification_stats().unwrap().n >= 1);
}

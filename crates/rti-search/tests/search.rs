use rti_core::experiment::{SearchMethod, SearchParams};
use rti_core::PhysicsParams;
use rti_search::common::SearchInput;
use rti_search::{run, Budget};
use rti_sim::{tracks, Sim, TrackGeom};

fn sim(name: &str) -> Sim {
    let t = tracks::builtin()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap();
    Sim::new(PhysicsParams::default(), TrackGeom::new(t))
}

fn input<'a>(sim: &'a Sim, params: &'a SearchParams, ticks: u64, seed: u64) -> SearchInput<'a> {
    SearchInput {
        sim,
        params,
        budget: Budget {
            max_ticks: ticks,
            max_wall_ms: Some(20_000),
        },
        seed,
        warm_start: None,
        policy: None,
    }
}

#[test]
fn every_method_respects_budget_and_reports() {
    let s = sim("s_curves");
    let params = SearchParams {
        max_ticks: Some(2500),
        population: Some(64),
        beam_width: Some(32),
        ..Default::default()
    };
    for m in [
        SearchMethod::RandomShooting,
        SearchMethod::Cem,
        SearchMethod::CmaEs,
        SearchMethod::Beam,
        SearchMethod::LocalOpt,
        SearchMethod::MapElites,
    ] {
        let out = run(m, &input(&s, &params, 1_000_000, 1)).unwrap();
        assert!(
            out.ticks > 0 && out.ticks <= 1_000_000 + 2_500 * 64 * 2,
            "{m:?} ticks {}",
            out.ticks
        );
        assert!(!out.history.is_empty());
        assert_eq!(out.method, m.name());
        assert!(
            out.best_result.progress > 50.0,
            "{m:?} made no progress: {:?}",
            out.best_result
        );
        eprintln!(
            "{:>16}: finished={} {} ms in {} evals / {} ticks",
            m.name(),
            out.best_result.finished,
            out.best_result.time_ms,
            out.evaluations,
            out.ticks
        );
    }
}

#[test]
fn strong_methods_finish_s_curves() {
    let s = sim("s_curves");
    let params = SearchParams {
        max_ticks: Some(2500),
        ..Default::default()
    };
    for m in [
        SearchMethod::Cem,
        SearchMethod::Beam,
        SearchMethod::LocalOpt,
    ] {
        let out = run(m, &input(&s, &params, 5_000_000, 1)).unwrap();
        assert!(
            out.best_result.finished,
            "{m:?} did not finish: {:?}",
            out.best_result
        );
        assert!(
            out.best_result.time_ms < 20_000,
            "{m:?} too slow: {}",
            out.best_result.time_ms
        );
    }
}

#[test]
fn beam_finishes_hairpin() {
    let s = sim("hairpin");
    let params = SearchParams {
        max_ticks: Some(3000),
        ..Default::default()
    };
    let out = run(SearchMethod::Beam, &input(&s, &params, 5_000_000, 1)).unwrap();
    assert!(out.best_result.finished, "{:?}", out.best_result);
}

#[test]
fn search_is_deterministic_for_seed() {
    let s = sim("s_curves");
    let params = SearchParams {
        max_ticks: Some(1500),
        population: Some(32),
        ..Default::default()
    };
    let a = run(SearchMethod::Cem, &input(&s, &params, 200_000, 9)).unwrap();
    let b = run(SearchMethod::Cem, &input(&s, &params, 200_000, 9)).unwrap();
    assert_eq!(a.best_actions, b.best_actions);
    assert_eq!(a.best_objective, b.best_objective);
}

#[test]
fn local_opt_improves_warm_start() {
    let s = sim("s_curves");
    let params = SearchParams {
        max_ticks: Some(2000),
        population: Some(32),
        ..Default::default()
    };
    let first = run(
        SearchMethod::RandomShooting,
        &input(&s, &params, 300_000, 2),
    )
    .unwrap();
    let mut inp = input(&s, &params, 600_000, 3);
    inp.warm_start = Some(&first.best_actions);
    let second = run(SearchMethod::LocalOpt, &inp).unwrap();
    assert!(
        second.best_objective <= first.best_objective,
        "{} vs {}",
        second.best_objective,
        first.best_objective
    );
}

#[test]
fn policy_method_requires_model() {
    let s = sim("straight");
    let params = SearchParams::default();
    assert!(run(SearchMethod::Policy, &input(&s, &params, 1000, 0)).is_err());
}

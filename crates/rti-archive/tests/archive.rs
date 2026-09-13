use rti_archive::{Archive, Finding, TaskRow};
use rti_core::trajectory::Divergence;
use rti_core::{
    Action, CarState, ContentHash, CostRecord, ExperimentSpec, PhysicsParams, Provenance,
    RunResult, SearchMethod, Surface, Track, Trajectory, ValueScore,
};

fn track(name: &str) -> Track {
    Track::from_segments(
        name,
        8.0,
        &[(0.0, 100.0, Surface::Asphalt), (90.0, 30.0, Surface::Dirt)],
    )
}

fn traj(t: &Track, time_ms: u32, finished: bool, progress: f32, world: &str) -> Trajectory {
    Trajectory {
        track_hash: t.hash(),
        track_name: t.name.clone(),
        world: world.into(),
        actions: vec![Action::full_gas(); (time_ms / 10) as usize],
        states: vec![CarState::default(); 3],
        result: RunResult {
            finished,
            time_ms,
            ticks: time_ms / 10,
            progress,
            ..Default::default()
        },
        method: "cem".into(),
        parent: None,
    }
}

#[test]
fn tracks_roundtrip() {
    let a = Archive::open_in_memory().unwrap();
    let t = track("t1");
    let h = a.upsert_track(&t).unwrap();
    let h2 = a.upsert_track(&t).unwrap();
    assert_eq!(h, h2);
    let rows = a.tracks().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "t1");
    assert!(rows[0].length_m > 100.0);
    let back = a.track_by_name("t1").unwrap().unwrap();
    assert_eq!(back, t);
    assert!(a.track_by_name("nope").unwrap().is_none());
    assert_eq!(a.track_by_hash(&h).unwrap().unwrap(), t);
}

#[test]
fn physics_current_defaults_and_switches() {
    let a = Archive::open_in_memory().unwrap();
    let (h, p) = a.current_physics().unwrap();
    assert_eq!(p, PhysicsParams::default());
    assert_eq!(h, PhysicsParams::default().hash());
    let p2 = PhysicsParams {
        grip: 30.0,
        ..Default::default()
    };
    let h2 = a
        .insert_physics(&p2, "more grip", Some("exp1"), Some(0.5))
        .unwrap();
    assert_eq!(a.current_physics().unwrap().0, h);
    a.set_current_physics(&h2).unwrap();
    let (h3, p3) = a.current_physics().unwrap();
    assert_eq!(h3, h2);
    assert_eq!(p3, p2);
    assert_eq!(a.physics_by_hash(&h2).unwrap().unwrap(), p2);
    assert!(a
        .physics_by_hash(&ContentHash("nope".into()))
        .unwrap()
        .is_none());
    let hist = a.physics_history(10).unwrap();
    assert_eq!(hist.len(), 2);
    assert!(hist.iter().filter(|r| r.is_current).count() == 1);
    assert!(a
        .set_current_physics(&ContentHash("missing".into()))
        .is_err());
}

#[test]
fn experiment_lifecycle() {
    let a = Archive::open_in_memory().unwrap();
    let spec = ExperimentSpec::Search {
        track: "t1".into(),
        method: SearchMethod::Cem,
        budget_ticks: 1000,
        seed: 1,
        params: Default::default(),
        warm_start: None,
        physics: None,
    };
    let prov = Provenance::now();
    let id = a.begin_experiment(&spec, None, Some(1), &prov).unwrap();
    let e = a.experiment(&id).unwrap().unwrap();
    assert_eq!(e.status, "running");
    assert_eq!(e.kind, "search");
    assert_eq!(e.method.as_deref(), Some("cem"));
    assert_eq!(e.track.as_deref(), Some("t1"));
    assert_eq!(e.cycle, Some(1));
    let cost = CostRecord {
        sim_ticks: 1000,
        wall_ms: 5,
        ..Default::default()
    };
    let value = ValueScore {
        race_improvement: 0.5,
        ..Default::default()
    };
    a.finish_experiment(
        &id,
        "done",
        &serde_json::json!({"best_time_ms": 1234, "finished": true}),
        &cost,
        Some(&value),
        Some(0.5),
        "did it",
    )
    .unwrap();
    let e = a.experiment(&id).unwrap().unwrap();
    assert_eq!(e.status, "done");
    assert_eq!(e.cost.unwrap().sim_ticks, 1000);
    assert_eq!(e.value_total, Some(0.5));
    assert_eq!(e.summary.as_deref(), Some("did it"));
    assert_eq!(e.result.unwrap()["best_time_ms"], 1234);
    assert!(e.finished_at.is_some());
    assert_eq!(a.recent_experiments(10).unwrap().len(), 1);
    assert_eq!(a.experiments_by_cycle(1).unwrap().len(), 1);
    assert_eq!(a.experiments_by_cycle(2).unwrap().len(), 0);
    assert_eq!(a.experiment_count().unwrap(), 1);
    assert!(a
        .finish_experiment(
            "nope",
            "done",
            &serde_json::Value::Null,
            &cost,
            None,
            None,
            ""
        )
        .is_err());
}

#[test]
fn trajectories_best_ordering() {
    let a = Archive::open_in_memory().unwrap();
    let t = track("t1");
    a.upsert_track(&t).unwrap();
    let h_slow = a
        .insert_trajectory(&traj(&t, 5000, true, 150.0, "sim:abc"), Some("e1"))
        .unwrap();
    let h_fast = a
        .insert_trajectory(&traj(&t, 4000, true, 150.0, "sim:abc"), Some("e1"))
        .unwrap();
    let h_dnf = a
        .insert_trajectory(&traj(&t, 9000, false, 80.0, "sim:abc"), None)
        .unwrap();
    let h_dnf2 = a
        .insert_trajectory(&traj(&t, 9000, false, 120.0, "sim:abc"), None)
        .unwrap();
    let h_oracle = a
        .insert_trajectory(&traj(&t, 4500, true, 150.0, "oracle:hidden_sim"), None)
        .unwrap();
    // idempotent
    assert_eq!(
        a.insert_trajectory(&traj(&t, 4000, true, 150.0, "sim:abc"), None)
            .unwrap(),
        h_fast
    );
    assert_eq!(a.trajectory_count().unwrap(), 5);

    let best = a.best_trajectories("t1", "sim", 10).unwrap();
    let hashes: Vec<&str> = best.iter().map(|r| r.hash.as_str()).collect();
    assert_eq!(
        hashes,
        vec![
            h_fast.as_str(),
            h_slow.as_str(),
            h_dnf2.as_str(),
            h_dnf.as_str()
        ]
    );
    assert!(best[0].has_states);
    assert_eq!(best[0].method.as_deref(), Some("cem"));

    let all = a.best_trajectories("t1", "", 10).unwrap();
    assert_eq!(all.len(), 5);
    assert_eq!(all[1].hash, h_oracle.as_str());

    assert_eq!(
        a.best_time("t1", "sim").unwrap(),
        Some((h_fast.clone(), 4000))
    );
    assert_eq!(
        a.best_time("t1", "oracle").unwrap(),
        Some((h_oracle.clone(), 4500))
    );
    assert_eq!(a.best_time("t2", "").unwrap(), None);

    let back = a.trajectory(&h_fast).unwrap().unwrap();
    assert_eq!(back.result.time_ms, 4000);
    assert_eq!(back.states.len(), 3);
    assert!(a
        .trajectory(&ContentHash("deadbeef".into()))
        .unwrap()
        .is_none());
    assert_eq!(a.trajectories_for_experiment("e1").unwrap().len(), 2);
    assert_eq!(a.trajectory_row(&h_dnf).unwrap().unwrap().progress, 80.0);
}

#[test]
fn verifications_and_stats() {
    let a = Archive::open_in_memory().unwrap();
    let h = ContentHash("t".into());
    let d1 = Divergence {
        ticks_compared: 100,
        mean_pos_err: 1.0,
        max_pos_err: 3.0,
        time_ms_a: 4000,
        time_ms_b: 4100,
        both_finished: true,
        ..Default::default()
    };
    let d2 = Divergence {
        ticks_compared: 100,
        mean_pos_err: 3.0,
        max_pos_err: 6.0,
        time_ms_a: 4000,
        time_ms_b: 3900,
        both_finished: true,
        ..Default::default()
    };
    a.insert_verification(
        &h,
        "hidden_sim",
        Some(&ContentHash("o".into())),
        &d1,
        Some("e1"),
    )
    .unwrap();
    a.insert_verification(&h, "hidden_sim", None, &d2, None)
        .unwrap();
    let s = a.verification_stats().unwrap();
    assert_eq!(s.n, 2);
    assert!((s.mean_pos_err - 2.0).abs() < 1e-9);
    assert!((s.mean_abs_time_diff_ms - 100.0).abs() < 1e-9);
    let recent = a.recent_verifications(5).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(
        recent
            .iter()
            .filter(|r| r.oracle_trajectory_hash.is_some())
            .count(),
        1
    );
    assert_eq!(recent[0].divergence["ticks_compared"], 100);
    let empty = Archive::open_in_memory().unwrap();
    assert_eq!(empty.verification_stats().unwrap().n, 0);
}

#[test]
fn models() {
    let a = Archive::open_in_memory().unwrap();
    let h1 = a.cas.put_bytes(b"weights1").unwrap();
    let h2 = a.cas.put_bytes(b"weights2").unwrap();
    a.insert_model(
        "policy",
        &h1,
        &serde_json::json!({"hidden": 64}),
        &serde_json::json!({"loss": 0.5}),
        None,
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    a.insert_model(
        "policy",
        &h2,
        &serde_json::json!({"hidden": 128}),
        &serde_json::json!({"loss": 0.4}),
        Some("e"),
    )
    .unwrap();
    a.insert_model(
        "value",
        &a.cas.put_bytes(b"v").unwrap(),
        &serde_json::json!({}),
        &serde_json::json!({}),
        None,
    )
    .unwrap();
    assert_eq!(a.models("policy", 10).unwrap().len(), 2);
    assert_eq!(a.latest_model("policy").unwrap().unwrap().hash, h2.as_str());
    assert_eq!(
        a.latest_model("policy").unwrap().unwrap().meta["hidden"],
        128
    );
    assert!(a.latest_model("nope").unwrap().is_none());
}

#[test]
fn findings_and_hypotheses() {
    let a = Archive::open_in_memory().unwrap();
    let id = a
        .insert_finding(&Finding {
            kind: "insight".into(),
            title: "Brake later".into(),
            body: "…".into(),
            confidence: 0.7,
            experiment_ids: vec!["e1".into()],
            tags: vec!["hairpin".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(!id.is_empty());
    a.insert_finding(&Finding {
        id: "custom".into(),
        kind: "anomaly".into(),
        title: "x".into(),
        body: "y".into(),
        confidence: 0.2,
        ..Default::default()
    })
    .unwrap();
    let all = a.findings(10).unwrap();
    assert_eq!(all.len(), 2);
    let f = a.finding(&id).unwrap().unwrap();
    assert_eq!(f.tags, vec!["hairpin"]);
    assert_eq!(f.experiment_ids, vec!["e1"]);
    assert!((f.confidence - 0.7).abs() < 1e-6);
    assert_eq!(a.findings_by_kind("anomaly", 10).unwrap()[0].id, "custom");

    let hid = a
        .insert_hypothesis("later braking is faster", "physics", Some(3))
        .unwrap();
    assert_eq!(a.hypotheses(Some("open"), 10).unwrap().len(), 1);
    a.set_hypothesis_status(&hid, "supported").unwrap();
    assert_eq!(a.hypotheses(Some("open"), 10).unwrap().len(), 0);
    let all = a.hypotheses(None, 10).unwrap();
    assert_eq!(all[0].status, "supported");
    assert_eq!(all[0].cycle, Some(3));
    assert!(a.set_hypothesis_status("nope", "x").is_err());
}

#[test]
fn ledger_totals_and_llm_calls() {
    let a = Archive::open_in_memory().unwrap();
    a.ledger_add(
        "experiment",
        "e1",
        &CostRecord {
            sim_ticks: 100,
            wall_ms: 10,
            llm_usd: 0.01,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    a.ledger_add(
        "experiment",
        "e2",
        &CostRecord {
            sim_ticks: 200,
            wall_ms: 20,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    a.ledger_add(
        "llm",
        "c1",
        &CostRecord {
            llm_calls: 1,
            llm_prompt_tokens: 500,
            llm_completion_tokens: 50,
            llm_usd: 0.02,
            ..Default::default()
        },
        "researcher",
    )
    .unwrap();
    let t = a.ledger_totals().unwrap();
    assert_eq!(t.sim_ticks, 300);
    assert_eq!(t.wall_ms, 30);
    assert_eq!(t.llm_calls, 1);
    assert_eq!(t.llm_prompt_tokens, 500);
    assert!((t.llm_usd - 0.03).abs() < 1e-9);
    let by = a.ledger_by_category().unwrap();
    assert_eq!(by.len(), 2);
    assert_eq!(by[0].0, "experiment");
    assert_eq!(by[0].1.sim_ticks, 300);
    assert_eq!(by[1].1.llm_calls, 1);
    assert_eq!(
        Archive::open_in_memory().unwrap().ledger_totals().unwrap(),
        CostRecord::default()
    );

    a.insert_llm_call(
        "researcher",
        "z-ai/glm-5.3-flash",
        1000,
        100,
        0.0001,
        900,
        Some(1),
        None,
        true,
        None,
    )
    .unwrap();
    a.insert_llm_call(
        "critic",
        "z-ai/glm-5.3",
        2000,
        200,
        0.003,
        1500,
        Some(1),
        Some("e1"),
        false,
        Some("timeout"),
    )
    .unwrap();
    assert!((a.llm_spend_total().unwrap() - 0.0031).abs() < 1e-9);
    let stats = a.llm_call_stats().unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].model, "z-ai/glm-5.3");
    assert_eq!(stats[0].prompt_tokens, 2000);
    let recent = a.recent_llm_calls(10).unwrap();
    assert_eq!(recent.len(), 2);
    assert!(recent
        .iter()
        .any(|r| r.error.as_deref() == Some("timeout") && !r.ok));
}

#[test]
fn tasks_queue() {
    let a = Archive::open_in_memory().unwrap();
    let low = a
        .insert_task(&TaskRow {
            kind: "coding".into(),
            title: "low".into(),
            description: "d".into(),
            priority: 1,
            ..Default::default()
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let high = a
        .insert_task(&TaskRow {
            kind: "verify".into(),
            title: "high".into(),
            description: "d".into(),
            priority: 5,
            cycle: Some(2),
            ..Default::default()
        })
        .unwrap();
    let p = a.pending_tasks().unwrap();
    assert_eq!(
        p.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec![high.as_str(), low.as_str()]
    );
    a.update_task(&high, "running", None, None, Some("rti/task-high"))
        .unwrap();
    assert_eq!(a.pending_tasks().unwrap().len(), 1);
    a.update_task(
        &high,
        "merged",
        Some(&serde_json::json!({"ok": true})),
        Some(&CostRecord {
            llm_usd: 0.5,
            ..Default::default()
        }),
        None,
    )
    .unwrap();
    let t = a.task(&high).unwrap().unwrap();
    assert_eq!(t.status, "merged");
    assert_eq!(t.branch.as_deref(), Some("rti/task-high"));
    assert_eq!(t.result_json.unwrap()["ok"], true);
    assert!((t.cost.unwrap().llm_usd - 0.5).abs() < 1e-9);
    assert_eq!(a.tasks(10).unwrap().len(), 2);
    assert!(a.update_task("nope", "x", None, None, None).is_err());
}

#[test]
fn cycles() {
    let a = Archive::open_in_memory().unwrap();
    assert!(a.last_cycle().unwrap().is_none());
    assert_eq!(a.cycle_count().unwrap(), 0);
    let c1 = a.begin_cycle("llm", "glm").unwrap();
    let c2 = a.begin_cycle("scripted", "-").unwrap();
    assert_eq!((c1, c2), (1, 2));
    a.finish_cycle(
        c1,
        "first",
        &CostRecord {
            llm_usd: 0.1,
            ..Default::default()
        },
    )
    .unwrap();
    let last = a.last_cycle().unwrap().unwrap();
    assert_eq!(last.id, 2);
    assert!(last.finished_at.is_none());
    let recent = a.recent_cycles(10).unwrap();
    assert_eq!(recent[1].summary.as_deref(), Some("first"));
    assert!((recent[1].cost.as_ref().unwrap().llm_usd - 0.1).abs() < 1e-9);
    assert_eq!(a.cycle_count().unwrap(), 2);
    assert!(a.finish_cycle(99, "", &CostRecord::default()).is_err());
}

#[test]
fn query_json_and_status() {
    let a = Archive::open_in_memory().unwrap();
    a.upsert_track(&track("t1")).unwrap();
    a.ledger_add(
        "experiment",
        "e1",
        &CostRecord {
            sim_ticks: 5,
            ..Default::default()
        },
        "n",
    )
    .unwrap();
    let rows = a
        .query_json("SELECT name, length_m, n_nodes FROM tracks")
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "t1");
    assert!(rows[0]["length_m"].is_f64());
    assert!(rows[0]["n_nodes"].is_i64());
    let rows = a.query_json("select sum(sim_ticks) AS s, count(*) AS n, TRUE AS b, NULL AS z, now() AS t, 1.5::FLOAT AS f, 2.25::DECIMAL(10,2) AS d from ledger").unwrap();
    assert_eq!(rows[0]["s"], 5);
    assert_eq!(rows[0]["n"], 1);
    assert_eq!(rows[0]["b"], true);
    assert!(rows[0]["z"].is_null());
    assert!(rows[0]["t"].is_string());
    assert!((rows[0]["f"].as_f64().unwrap() - 1.5).abs() < 1e-6);
    assert!((rows[0]["d"].as_f64().unwrap() - 2.25).abs() < 1e-6);
    assert!(a.query_json("DELETE FROM tracks").is_err());
    assert!(a
        .query_json("  WITH x AS (SELECT 1 AS v) SELECT v FROM x")
        .is_ok());
    let st = a.status().unwrap();
    assert_eq!(st["tracks"], 1);
    assert_eq!(st["ledger"]["sim_ticks"], 5);
    assert!(Archive::schema_doc().contains("experiments("));
}

#[test]
fn method_stats_aggregates() {
    let a = Archive::open_in_memory().unwrap();
    let prov = Provenance::now();
    let mk = |method: SearchMethod, track: &str| ExperimentSpec::Search {
        track: track.into(),
        method,
        budget_ticks: 10,
        seed: 0,
        params: Default::default(),
        warm_start: None,
        physics: None,
    };
    let e1 = a
        .begin_experiment(&mk(SearchMethod::Cem, "t1"), None, None, &prov)
        .unwrap();
    let e2 = a
        .begin_experiment(&mk(SearchMethod::Cem, "t1"), None, None, &prov)
        .unwrap();
    let e3 = a
        .begin_experiment(&mk(SearchMethod::Beam, "t1"), None, None, &prov)
        .unwrap();
    let e4 = a
        .begin_experiment(&mk(SearchMethod::Beam, "t1"), None, None, &prov)
        .unwrap();
    let c = |ticks: u64, usd: f64| CostRecord {
        sim_ticks: ticks,
        wall_ms: 1,
        llm_usd: usd,
        ..Default::default()
    };
    a.finish_experiment(
        &e1,
        "done",
        &serde_json::json!({"finished": true, "best_time_ms": 5000}),
        &c(100, 0.0),
        None,
        None,
        "",
    )
    .unwrap();
    a.finish_experiment(
        &e2,
        "done",
        &serde_json::json!({"finished": true, "best_time_ms": 4800}),
        &c(200, 0.01),
        None,
        None,
        "",
    )
    .unwrap();
    a.finish_experiment(
        &e3,
        "done",
        &serde_json::json!({"finished": false}),
        &c(50, 0.0),
        None,
        None,
        "",
    )
    .unwrap();
    a.finish_experiment(
        &e4,
        "failed",
        &serde_json::Value::Null,
        &c(1, 0.0),
        None,
        None,
        "",
    )
    .unwrap();
    let stats = a.method_stats().unwrap();
    assert_eq!(stats.len(), 2);
    let beam = stats.iter().find(|s| s.method == "beam").unwrap();
    assert_eq!(
        (
            beam.n_runs,
            beam.finished_runs,
            beam.best_time_ms,
            beam.sim_ticks
        ),
        (1, 0, None, 50)
    );
    let cem = stats.iter().find(|s| s.method == "cem").unwrap();
    assert_eq!(
        (
            cem.n_runs,
            cem.finished_runs,
            cem.best_time_ms,
            cem.sim_ticks
        ),
        (2, 2, Some(4800), 300)
    );
    assert!((cem.llm_usd - 0.01).abs() < 1e-9);
    assert_eq!(cem.track, "t1");
}

#[test]
fn cas_basics() {
    let a = Archive::open_in_memory().unwrap();
    let h = a.cas.put_bytes(b"hello").unwrap();
    assert_eq!(h, ContentHash::of_bytes(b"hello"));
    assert!(a.cas.exists(&h));
    assert_eq!(a.cas.get_bytes(&h).unwrap(), b"hello");
    assert_eq!(a.cas.put_bytes(b"hello").unwrap(), h);
    assert!(a.cas.path(&h).starts_with(a.cas.root()));
    let hj = a.cas.put_json(&serde_json::json!({"a": 1})).unwrap();
    let v: serde_json::Value = a.cas.get_json(&hj).unwrap();
    assert_eq!(v["a"], 1);
    assert!(a.cas.get_bytes(&ContentHash("00missing".into())).is_err());
    assert_eq!(a.cas.stats().unwrap().0, 2);
}

#[test]
fn on_disk_open_persists() {
    let dir = tempfile::tempdir().unwrap();
    {
        let a = Archive::open(dir.path()).unwrap();
        a.upsert_track(&track("persist")).unwrap();
    }
    let a = Archive::open(dir.path()).unwrap();
    assert!(a.track_by_name("persist").unwrap().is_some());
    assert!(dir.path().join("rti.duckdb").exists());
    assert!(dir.path().join("cas").is_dir());
}

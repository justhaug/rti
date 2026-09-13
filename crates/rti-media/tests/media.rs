//! End-to-end media test: build two runs on a built-in track in a temp
//! project, detect the candidate, render + compose an MP4 (needs ffmpeg:
//! set RTI_FFMPEG or have it on PATH; skipped otherwise), narrate, archive.
use rti_core::config::InterestWeights;
use rti_core::experiment::{SearchMethod, SearchParams};
use rti_core::{ExperimentSpec, Provenance, RtiConfig};
use rti_media::compare::compare;
use rti_media::detect::scan;
use rti_media::pipeline::{produce, MediaConfig};
use rti_research::{run_experiment, Session};

fn session() -> (tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = RtiConfig::default();
    cfg.research.brain = "scripted".into();
    cfg.data_dir = dir.path().join("data");
    cfg.tracks_dir = dir.path().join("tracks");
    let s = Session::with_config(dir.path(), cfg).unwrap();
    (dir, s)
}

fn run(s: &Session, spec: ExperimentSpec) -> rti_research::ExperimentReport {
    let id = s
        .archive
        .begin_experiment(&spec, None, None, &Provenance::now())
        .unwrap();
    let r = run_experiment(s, &spec, &id).unwrap();
    s.archive
        .finish_experiment(&id, "done", &r.result, &r.cost, None, None, &r.summary)
        .unwrap();
    r
}

#[test]
fn detect_compare_render_compose() {
    let (_dir, s) = session();
    // two different finishing runs on s_curves (sim world)
    let p1 = SearchParams {
        max_ticks: Some(2500),
        macro_ticks: Some(30),
        beam_width: Some(32),
        ..Default::default()
    };
    let p2 = SearchParams {
        max_ticks: Some(2500),
        macro_ticks: Some(20),
        beam_width: Some(64),
        ..Default::default()
    };
    run(
        &s,
        ExperimentSpec::Search {
            track: "s_curves".into(),
            method: SearchMethod::Beam,
            budget_ticks: 3_000_000,
            seed: 1,
            params: p1,
            warm_start: None,
            physics: None,
        },
    );
    run(
        &s,
        ExperimentSpec::Search {
            track: "s_curves".into(),
            method: SearchMethod::Beam,
            budget_ticks: 3_000_000,
            seed: 2,
            params: p2,
            warm_start: None,
            physics: None,
        },
    );
    let cands = scan(&s, &InterestWeights::default(), false).unwrap();
    let c = cands
        .iter()
        .find(|c| c.track_name == "s_curves")
        .expect("candidate on s_curves");
    assert!(
        c.old_trajectory.is_some(),
        "expected a previous best: {c:?}"
    );
    assert!(c.interest.score > 0.0);

    let track = s.track("s_curves").unwrap();
    let new = s
        .archive
        .trajectory(&rti_core::ContentHash(c.new_trajectory.clone()))
        .unwrap()
        .unwrap();
    let old = s
        .archive
        .trajectory(&rti_core::ContentHash(c.old_trajectory.clone().unwrap()))
        .unwrap()
        .unwrap();
    let cmp = compare(&new, &old, track.length(), 8);
    assert_eq!(
        cmp.delta_ms,
        new.result.time_ms as i32 - old.result.time_ms as i32
    );
    assert!(!cmp.sections.is_empty());
    assert!(!cmp.bullets.is_empty());

    if rti_media::compose::ffmpeg_path().is_none() {
        eprintln!("ffmpeg not found; skipping render/compose");
        return;
    }
    let cfg = MediaConfig {
        require_verified: false,
        width: 540,
        height: 960,
        fps: 15,
        ..Default::default()
    };
    let row = produce(&s, &cfg, c).unwrap();
    assert_eq!(row.status, "review");
    assert!(row.frames > 60, "frames {}", row.frames);
    let path = std::path::Path::new(row.video_path.as_ref().unwrap());
    assert!(path.is_file());
    assert!(std::fs::metadata(path).unwrap().len() > 20_000);
    if let Some(d) = rti_media::compose::probe_duration(path) {
        assert!(d > 5.0, "duration {d}");
    }
    assert!(!row.title.is_empty());
    assert!(s.archive.media(10).unwrap().iter().any(|m| m.id == row.id));
    // idempotent: the same best is not produced twice
    let again = scan(&s, &InterestWeights::default(), false).unwrap();
    assert!(!again.iter().any(|x| x.new_trajectory == c.new_trajectory));
}

#[test]
fn overview_frame_renders() {
    let (_dir, s) = session();
    let params = SearchParams {
        max_ticks: Some(2000),
        ..Default::default()
    };
    run(
        &s,
        ExperimentSpec::Search {
            track: "hairpin".into(),
            method: SearchMethod::Beam,
            budget_ticks: 2_000_000,
            seed: 1,
            params,
            warm_start: None,
            physics: None,
        },
    );
    let (h, _) = s.archive.best_time("hairpin", "sim").unwrap().unwrap();
    let t = s.archive.trajectory(&h).unwrap().unwrap();
    let track = s.track("hairpin").unwrap();
    let f = rti_media::render::overview_frame(&track, &t, None, 400, 400);
    assert_eq!(f.px.len(), 400 * 400 * 3);
    // something non-background was drawn
    assert!(f.px.chunks(3).any(|p| p != rti_media::render::BG));
}

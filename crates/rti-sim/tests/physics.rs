use rti_core::{Action, PhysicsParams};
use rti_sim::{observe, rollout, rollout_batch, tracks, Sim, TrackGeom, OBS_DIM};

fn sim(name: &str) -> Sim {
    let t = tracks::builtin()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap();
    Sim::new(PhysicsParams::default(), TrackGeom::new(t))
}

#[test]
fn deterministic_replay() {
    let s = sim("s_curves");
    let acts: Vec<Action> = (0..3000)
        .map(|i| Action::new(((i as f32) * 0.01).sin() * 0.6, true, i % 97 == 0))
        .collect();
    let a = rollout(&s, &s.initial_state(), &acts, 3000, true);
    let b = rollout(&s, &s.initial_state(), &acts, 3000, true);
    assert_eq!(a.states, b.states);
    assert_eq!(a.result, b.result);
}

#[test]
fn savestate_resume_matches_full_run() {
    let s = sim("hairpin");
    let acts: Vec<Action> = (0..2000)
        .map(|i| {
            Action::new(
                if i > 700 && i < 1200 { -0.8 } else { 0.0 },
                i < 600 || i > 1100,
                i >= 600 && i <= 700,
            )
        })
        .collect();
    let full = rollout(&s, &s.initial_state(), &acts, 2000, true);
    let mid = full.states[900];
    let resumed = rollout(&s, &mid, &acts[900..], 2000, true);
    let n = resumed.states.len();
    assert_eq!(&full.states[900..900 + n], &resumed.states[..]);
}

#[test]
fn full_gas_finishes_straight() {
    let s = sim("straight");
    let acts = vec![Action::full_gas(); 6000];
    let r = rollout(&s, &s.initial_state(), &acts, 6000, false);
    assert!(r.result.finished, "{:?}", r.result);
    assert!(
        r.result.time_ms > 5000 && r.result.time_ms < 15000,
        "time {}",
        r.result.time_ms
    );
    assert!(
        r.result.max_speed > 40.0 && r.result.max_speed <= 120.0,
        "max speed {}",
        r.result.max_speed
    );
}

#[test]
fn full_gas_hits_wall_on_hairpin() {
    let s = sim("hairpin");
    let acts = vec![Action::full_gas(); 3000];
    let r = rollout(&s, &s.initial_state(), &acts, 3000, false);
    assert!(r.result.wall_hits > 0);
    assert!(r.result.progress < s.geom.total_len);
}

#[test]
fn steering_turns_left_positive() {
    let s = sim("straight");
    let mut st = s.initial_state();
    for _ in 0..200 {
        s.step(&mut st, Action::new(1.0, true, false));
    }
    assert!(st.heading > 0.05, "heading {}", st.heading);
    assert!(st.y > 0.0);
}

#[test]
fn observation_is_finite() {
    let s = sim("mixed_surface");
    let mut st = s.initial_state();
    for i in 0..500 {
        s.step(&mut st, Action::new((i as f32 * 0.02).sin(), true, false));
        let o = observe(&s.geom, &st);
        assert_eq!(o.len(), OBS_DIM);
        assert!(o.iter().all(|v| v.is_finite()));
    }
}

#[test]
fn batch_matches_single() {
    let s = sim("loop");
    let seqs: Vec<Vec<Action>> = (0..8)
        .map(|k| {
            (0..1500)
                .map(|i| Action::new(((i + k * 50) as f32 * 0.01).sin() * 0.5, true, false))
                .collect()
        })
        .collect();
    let (batch, ticks) = rollout_batch(&s, &s.initial_state(), &seqs, 1500);
    assert!(ticks > 0);
    for (k, r) in batch.iter().enumerate() {
        let single = rollout(&s, &s.initial_state(), &seqs[k], 1500, false);
        assert_eq!(r.result, single.result);
    }
}

#[test]
fn generated_track_is_valid_and_drivable() {
    let t = tracks::generate("gen_test", 42, 6, 8.0);
    t.validate().unwrap();
    let s = Sim::new(PhysicsParams::default(), TrackGeom::new(t));
    let acts = vec![Action::new(0.0, true, false); 500];
    let r = rollout(&s, &s.initial_state(), &acts, 500, false);
    assert!(r.result.progress > 10.0);
}

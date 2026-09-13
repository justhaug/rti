use rti_core::action::expand_runs;
use rti_core::{Action, PhysicsParams, RunResult, Trajectory};
use rti_oracle::hidden::HiddenSim;
use rti_oracle::protocol::{Request, Response, TmResult, TmState};
use rti_oracle::tm2020::Tm2020;
use rti_oracle::{verify, Oracle};
use rti_sim::{tracks, Sim, TrackGeom};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

fn track(name: &str) -> rti_core::Track {
    tracks::builtin()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap()
}

#[test]
fn hidden_sim_disagrees_with_public_sim() {
    let t = track("hairpin");
    let sim = Sim::new(PhysicsParams::default(), TrackGeom::new(t.clone()));
    let acts: Vec<Action> = (0..2000)
        .map(|i| {
            Action::new(
                if (700..1300).contains(&i) { -0.7 } else { 0.0 },
                !(600..720).contains(&i),
                (600..720).contains(&i),
            )
        })
        .collect();
    let ro = rti_sim::rollout(&sim, &sim.initial_state(), &acts, 2000, false);
    let traj = Trajectory {
        track_hash: t.hash(),
        track_name: t.name.clone(),
        world: "sim".into(),
        actions: acts,
        states: vec![],
        result: ro.result,
        method: "test".into(),
        parent: None,
    };
    let oracle = HiddenSim::from_seed(7);
    let v = verify(&oracle, &sim, &traj).unwrap();
    assert!(v.divergence.ticks_compared > 100);
    assert!(
        v.divergence.mean_pos_err > 0.05,
        "expected divergence, got {:?}",
        v.divergence
    );
    assert!(v.divergence.mean_pos_err.is_finite());
}

/// A mock bridge that answers the protocol using the hidden sim, exercising
/// the TM2020 client end to end (serialisation, framing, projection).
#[test]
fn tm2020_client_roundtrip_with_mock_bridge() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut t = track("s_curves");
    t.tm_map_uid = Some("mock-uid".into());
    let t_server = t.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut w = stream;
            let hidden = HiddenSim::from_seed(3);
            let t_server = t_server.clone();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let req: Request = serde_json::from_str(line.trim()).unwrap();
                let resp = match req {
                    Request::Hello { .. } => Response {
                        game: Some("mock".into()),
                        ..Response::ok()
                    },
                    Request::Ping | Request::LoadMap { .. } => Response::ok(),
                    Request::Run {
                        inputs, max_ticks, ..
                    } => {
                        let acts = expand_runs(&inputs);
                        let run = hidden.run(&t_server, &acts, max_ticks).unwrap();
                        let states = run
                            .states
                            .iter()
                            .map(|s| TmState {
                                tick: s.tick,
                                pos: [s.x, 0.0, s.y],
                                vel: [s.vx, 0.0, s.vy],
                                yaw: s.heading,
                                finished: s.finished,
                                race_time_ms: s.finish_tick * 10,
                                cp: s.next_checkpoint,
                                ..Default::default()
                            })
                            .collect();
                        Response {
                            result: Some(TmResult {
                                finished: run.result.finished,
                                race_time_ms: run.result.time_ms,
                                ticks: run.result.ticks,
                                checkpoints: run.result.checkpoints_hit,
                            }),
                            states,
                            ..Response::ok()
                        }
                    }
                };
                let mut s = serde_json::to_string(&resp).unwrap();
                s.push('\n');
                w.write_all(s.as_bytes()).unwrap();
            }
        }
    });
    let client = Tm2020::new("127.0.0.1", port, 10);
    client.available().unwrap();
    let acts: Vec<Action> = (0..800)
        .map(|i| Action::new(((i as f32) * 0.02).sin() * 0.3, true, false))
        .collect();
    let run = client.run(&t, &acts, 800).unwrap();
    let direct = HiddenSim::from_seed(3).run(&t, &acts, 800).unwrap();
    assert!(run.states.len() > 100);
    assert_eq!(run.states.len(), direct.states.len());
    let a = run.states.last().unwrap();
    let b = direct.states.last().unwrap();
    assert!((a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3);
    let _: RunResult = run.result;
}

#[test]
fn unreachable_bridge_gives_helpful_error() {
    let client = Tm2020::new("127.0.0.1", 1, 1);
    let err = client.available().unwrap_err().to_string();
    assert!(err.contains("docs/oracle.md"), "{err}");
}

use rand::SeedableRng;
use rti_core::{Action, PhysicsParams, RunResult, Trajectory};
use rti_nn::{
    behavior_clone, policy_rollout, Activation, BcConfig, Loss, Mlp, PolicyBundle, TrainConfig,
};
use rti_sim::{observe, rollout, tracks, Sim, TrackGeom, OBS_DIM};

#[test]
fn learns_xor() {
    let xs = vec![
        vec![0.0, 0.0],
        vec![0.0, 1.0],
        vec![1.0, 0.0],
        vec![1.0, 1.0],
    ];
    let ys = vec![vec![0.0], vec![1.0], vec![1.0], vec![0.0]];
    let mut net = Mlp::new(&[2, 8, 1], Activation::Tanh, 1);
    let cfg = TrainConfig {
        epochs: 800,
        lr: 0.02,
        batch_size: 4,
        ..Default::default()
    };
    let hist = net.train(&xs, &ys, Loss::Mse, &cfg);
    let last = *hist.last().unwrap();
    assert!(last < 0.01, "xor loss {last}");
    for (x, y) in xs.iter().zip(&ys) {
        assert!((net.forward(x)[0] - y[0]).abs() < 0.2);
    }
}

#[test]
fn learns_sin() {
    let xs: Vec<Vec<f32>> = (0..200)
        .map(|i| vec![i as f32 / 200.0 * std::f32::consts::TAU - std::f32::consts::PI])
        .collect();
    let ys: Vec<Vec<f32>> = xs.iter().map(|x| vec![x[0].sin()]).collect();
    let mut net = Mlp::new(&[1, 16, 16, 1], Activation::Tanh, 3);
    let cfg = TrainConfig {
        epochs: 300,
        lr: 0.01,
        batch_size: 32,
        ..Default::default()
    };
    let hist = net.train(&xs, &ys, Loss::Mse, &cfg);
    assert!(hist[0] > *hist.last().unwrap());
    assert!(*hist.last().unwrap() < 0.01, "sin loss {:?}", hist.last());
}

#[test]
fn numerical_gradient_matches() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(9);
    let xs: Vec<Vec<f32>> = (0..5)
        .map(|_| {
            (0..4)
                .map(|_| rand::Rng::random_range(&mut rng, -1.0..1.0))
                .collect()
        })
        .collect();
    for (loss, ys) in [
        (
            Loss::Mse,
            (0..5)
                .map(|_| {
                    (0..3)
                        .map(|_| rand::Rng::random_range(&mut rng, -1.0..1.0))
                        .collect()
                })
                .collect::<Vec<Vec<f32>>>(),
        ),
        (
            Loss::PolicyLoss,
            (0..5)
                .map(|i| {
                    vec![
                        rand::Rng::random_range(&mut rng, -1.0..1.0),
                        (i % 2) as f32,
                        ((i + 1) % 2) as f32,
                    ]
                })
                .collect(),
        ),
    ] {
        for act in [Activation::Tanh, Activation::Relu] {
            let net = Mlp::new(&[4, 5, 3], act, 4);
            let (nw, nb) = net.numerical_grad(&xs, &ys, loss, 1e-2);
            let (aw, ab) = net.analytic_grad(&xs, &ys, loss);
            for (n, a) in nw
                .iter()
                .flatten()
                .zip(aw.iter().flatten())
                .chain(nb.iter().flatten().zip(ab.iter().flatten()))
            {
                assert!(
                    (n - a).abs() < 2e-2 + 5e-2 * a.abs(),
                    "{loss:?}/{act:?}: numerical {n} vs analytic {a}"
                );
            }
        }
    }
}

#[test]
fn same_seed_same_result() {
    let xs: Vec<Vec<f32>> = (0..64).map(|i| vec![i as f32 / 64.0]).collect();
    let ys: Vec<Vec<f32>> = xs.iter().map(|x| vec![x[0] * x[0]]).collect();
    let cfg = TrainConfig {
        epochs: 5,
        ..Default::default()
    };
    let mut a = Mlp::new(&[1, 8, 1], Activation::Relu, 5);
    let mut b = Mlp::new(&[1, 8, 1], Activation::Relu, 5);
    let ha = a.train(&xs, &ys, Loss::Mse, &cfg);
    let hb = b.train(&xs, &ys, Loss::Mse, &cfg);
    assert_eq!(ha, hb);
    assert_eq!(a.weights, b.weights);
}

/// Hand-made centerline-following controller used as the demonstrator.
/// `noise` perturbs the *executed* steering (DART-style) so the dataset
/// covers recovery states; labels stay unbiased because the noise is zero-mean.
fn demo_trajectory(sim: &Sim, max_ticks: u32, noise: f32, seed: u64) -> Trajectory {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut s = sim.initial_state();
    let mut actions = Vec::new();
    while !sim.is_terminal(&s, max_ticks) {
        let o = observe(&sim.geom, &s);
        let lateral = o[5];
        let heading_err = o[6];
        let ahead = o[8 + 3]; // heading of the road 16 m ahead relative to car
        let steer = (-0.6 * lateral - 1.0 * heading_err + 2.0 * ahead).clamp(-1.0, 1.0);
        // crude speed management: slow down for curvature 32-64 m ahead
        let curv = o[8 + 3 * 2 + 1].abs().max(o[8 + 3 * 3 + 1].abs()) / 20.0;
        let v_target = if curv > 1e-4 {
            (18.0 / curv).sqrt().min(60.0)
        } else {
            60.0
        };
        let v = o[0] * 60.0;
        let brake = v > v_target + 3.0;
        let gas = v < v_target;
        let g: f32 = rand_distr::Distribution::sample(&rand_distr::StandardNormal, &mut rng);
        let a = Action::new(steer + g * noise, gas, brake).clamped();
        sim.step(&mut s, a);
        actions.push(a);
    }
    let ro = rollout(sim, &sim.initial_state(), &actions, max_ticks, false);
    Trajectory {
        track_hash: sim.geom.track.hash(),
        track_name: sim.geom.track.name.clone(),
        world: "sim:test".into(),
        actions,
        states: vec![],
        result: ro.result,
        method: "demo".into(),
        parent: None,
    }
}

#[test]
fn behavior_cloning_learns_demo() {
    let t = tracks::builtin()
        .into_iter()
        .find(|t| t.name == "s_curves")
        .unwrap();
    let sim = Sim::new(PhysicsParams::default(), TrackGeom::new(t));
    let demo = demo_trajectory(&sim, 3000, 0.0, 0);
    assert!(demo.result.finished, "demo only reached {:?}", demo.result);
    let noisy: Vec<Trajectory> = [(0.1, 1), (0.2, 2), (0.3, 3), (0.2, 4), (0.3, 5)]
        .iter()
        .map(|&(n, seed)| demo_trajectory(&sim, 3000, n, seed))
        .collect();
    let mut datasets = vec![(&sim, &demo)];
    datasets.extend(noisy.iter().map(|t| (&sim, t)));
    let cfg = BcConfig {
        epochs: 40,
        hidden: 64,
        seed: 1,
        value_head: true,
        lr: 3e-3,
    };
    let (bundle, m) = behavior_clone(&datasets, &cfg).unwrap();
    assert_eq!(
        m.n_samples,
        datasets.iter().map(|(_, t)| t.actions.len()).sum::<usize>()
    );
    assert!(
        m.final_loss < m.initial_loss * 0.5,
        "initial {} final {}",
        m.initial_loss,
        m.final_loss
    );
    assert!(m.value_loss.is_some());

    eprintln!(
        "demo {:?}\nmetrics init {} final {} value {:?} epochs {:?}",
        demo.result,
        m.initial_loss,
        m.final_loss,
        m.value_loss,
        &m.epoch_losses[m.epoch_losses.len() - 3..]
    );
    let (acts, ro) = policy_rollout(&sim, &bundle, 0.0, 3000, 0);
    eprintln!("policy {:?}", ro.result);
    let (_, ro2) = policy_rollout(&sim, &bundle, 0.05, 3000, 0);
    eprintln!("policy noisy {:?}", ro2.result);
    assert_eq!(acts.len() as u64, ro.ticks);
    assert!(
        ro.result.progress > 100.0,
        "policy only reached {:?}",
        ro.result
    );
    assert!(
        ro.result.wall_hits <= demo.result.wall_hits * 2 + 20,
        "policy {:?} vs demo {:?}",
        ro.result,
        demo.result
    );

    let bytes = bundle.to_bytes();
    let back = PolicyBundle::from_bytes(&bytes).unwrap();
    assert_eq!(back.obs_dim, OBS_DIM);
    let o = observe(&sim.geom, &sim.initial_state());
    assert_eq!(
        back.policy.act_deterministic(&o),
        bundle.policy.act_deterministic(&o)
    );
    let _: RunResult = ro.result;
}

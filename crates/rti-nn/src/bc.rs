use rand::{Rng, SeedableRng};
use rti_core::Trajectory;
use rti_sim::{observe, rollout, Sim, OBS_DIM};
use serde::{Deserialize, Serialize};

use crate::mlp::{Loss, TrainConfig};
use crate::policy::{Policy, PolicyBundle, ValueNet, VALUE_SCALE};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BcConfig {
    pub epochs: usize,
    pub hidden: usize,
    pub seed: u64,
    pub value_head: bool,
    pub lr: f32,
}

impl Default for BcConfig {
    fn default() -> Self {
        BcConfig {
            epochs: 30,
            hidden: 64,
            seed: 0,
            value_head: true,
            lr: 1e-3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BcMetrics {
    pub n_samples: usize,
    /// Held-out policy loss after training.
    pub final_loss: f32,
    /// Held-out policy loss before training (for "did it learn anything").
    pub initial_loss: f32,
    pub epoch_losses: Vec<f32>,
    /// Held-out value loss (MSE on normalised remaining time), if trained.
    pub value_loss: Option<f32>,
}

/// (observations, action targets, value targets), one row per tick.
pub type Dataset = (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<Vec<f32>>);

/// Re-simulate a trajectory and turn every tick into a training sample:
/// (observation, `[steer, gas, brake]`, `[remaining_ticks / VALUE_SCALE]`).
pub fn build_dataset(sim: &Sim, traj: &Trajectory) -> Dataset {
    let max_ticks = sim.geom.track.max_ticks.max(traj.actions.len() as u32);
    let ro = rollout(sim, &sim.initial_state(), &traj.actions, max_ticks, true);
    // states[i] is the state *before* actions[i] (states[0] = initial).
    let n = ro.states.len().saturating_sub(1).min(traj.actions.len());
    let end_tick = ro.final_state.tick as f32;
    let mut obs = Vec::with_capacity(n);
    let mut acts = Vec::with_capacity(n);
    let mut vals = Vec::with_capacity(n);
    for i in 0..n {
        let s = &ro.states[i];
        obs.push(observe(&sim.geom, s).to_vec());
        acts.push(traj.actions[i].to_vec().to_vec());
        vals.push(vec![(end_tick - s.tick as f32).max(0.0) / VALUE_SCALE]);
    }
    (obs, acts, vals)
}

/// Behaviour-clone a policy (and optionally a value head) from trajectories.
pub fn behavior_clone(
    datasets: &[(&Sim, &Trajectory)],
    cfg: &BcConfig,
) -> anyhow::Result<(PolicyBundle, BcMetrics)> {
    let mut xs = Vec::new();
    let mut ya = Vec::new();
    let mut yv = Vec::new();
    for (sim, traj) in datasets {
        let (o, a, v) = build_dataset(sim, traj);
        xs.extend(o);
        ya.extend(a);
        yv.extend(v);
    }
    anyhow::ensure!(
        !xs.is_empty(),
        "behaviour cloning needs at least one non-empty trajectory"
    );

    let mut rng = rand::rngs::StdRng::seed_from_u64(cfg.seed);
    let mut idx: Vec<usize> = (0..xs.len()).collect();
    for i in (1..idx.len()).rev() {
        let j = rng.random_range(0..=i);
        idx.swap(i, j);
    }
    let n_hold = (xs.len() / 10).max(if xs.len() > 1 { 1 } else { 0 });
    let (hold, train) = idx.split_at(n_hold);
    let pick = |ids: &[usize], src: &[Vec<f32>]| -> Vec<Vec<f32>> {
        ids.iter().map(|&i| src[i].clone()).collect()
    };
    let tx = pick(train, &xs);
    let ta = pick(train, &ya);
    let tv = pick(train, &yv);
    let hx = pick(hold, &xs);
    let ha = pick(hold, &ya);
    let hv = pick(hold, &yv);

    let tcfg = TrainConfig {
        epochs: cfg.epochs,
        lr: cfg.lr,
        seed: cfg.seed,
        ..Default::default()
    };
    let mut policy = Policy::new(cfg.hidden, cfg.seed);
    let initial_loss = policy.net.evaluate(&hx, &ha, Loss::PolicyLoss);
    let epoch_losses = policy.net.train(&tx, &ta, Loss::PolicyLoss, &tcfg);
    let final_loss = if hx.is_empty() {
        *epoch_losses.last().unwrap_or(&0.0)
    } else {
        policy.net.evaluate(&hx, &ha, Loss::PolicyLoss)
    };

    let mut value = None;
    let mut value_loss = None;
    if cfg.value_head {
        let mut vn = ValueNet::new(cfg.hidden, cfg.seed.wrapping_add(1));
        let losses = vn.net.train(&tx, &tv, Loss::Mse, &tcfg);
        value_loss = Some(if hx.is_empty() {
            *losses.last().unwrap_or(&0.0)
        } else {
            vn.net.evaluate(&hx, &hv, Loss::Mse)
        });
        value = Some(vn);
    }

    let metrics = BcMetrics {
        n_samples: xs.len(),
        final_loss,
        initial_loss,
        epoch_losses,
        value_loss,
    };
    let bundle = PolicyBundle {
        policy,
        value,
        obs_dim: OBS_DIM,
        meta: serde_json::json!({
            "kind": "behavior_cloning",
            "hidden": cfg.hidden,
            "epochs": cfg.epochs,
            "seed": cfg.seed,
            "n_trajectories": datasets.len(),
            "n_samples": xs.len(),
            "tracks": datasets.iter().map(|(_, t)| t.track_name.clone()).collect::<Vec<_>>(),
        }),
    };
    Ok((bundle, metrics))
}

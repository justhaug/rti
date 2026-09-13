use rand::Rng;
use rti_core::Action;
use rti_sim::OBS_DIM;
use serde::{Deserialize, Serialize};

use crate::mlp::{Activation, Mlp};

/// Value-net targets are remaining ticks divided by this constant.
pub const VALUE_SCALE: f32 = 6000.0;

#[inline]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Policy net: observation -> `[steer, gas_logit, brake_logit]`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Policy {
    pub net: Mlp,
}

impl Policy {
    pub fn new(hidden: usize, seed: u64) -> Policy {
        Policy {
            net: Mlp::new(&[OBS_DIM, hidden, hidden, 3], Activation::Tanh, seed),
        }
    }

    /// Stochastic action: gaussian steering noise and probabilistic
    /// gas/brake flips, both scaled by `noise` (0 = deterministic).
    pub fn act(&self, obs: &[f32], noise: f32, rng: &mut impl Rng) -> Action {
        let out = self.net.forward(obs);
        let mut steer = out[0].tanh();
        let mut gas = sigmoid(out[1]) > 0.5;
        let mut brake = sigmoid(out[2]) > 0.5;
        if noise > 0.0 {
            let g: f32 = rand_distr::Distribution::sample(&rand_distr::StandardNormal, rng);
            steer += g * noise;
            let flip_p = (noise * 0.25).clamp(0.0, 0.5);
            if rng.random::<f32>() < flip_p {
                gas = !gas;
            }
            if rng.random::<f32>() < flip_p {
                brake = !brake;
            }
        }
        Action::new(steer, gas, brake).clamped()
    }

    pub fn act_deterministic(&self, obs: &[f32]) -> Action {
        let out = self.net.forward(obs);
        Action::new(out[0].tanh(), sigmoid(out[1]) > 0.5, sigmoid(out[2]) > 0.5).clamped()
    }
}

/// Value net: observation -> normalised remaining time (`remaining_ticks / VALUE_SCALE`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValueNet {
    pub net: Mlp,
}

impl ValueNet {
    pub fn new(hidden: usize, seed: u64) -> ValueNet {
        ValueNet {
            net: Mlp::new(&[OBS_DIM, hidden, hidden, 1], Activation::Tanh, seed),
        }
    }

    /// Predicted remaining ticks.
    pub fn remaining_ticks(&self, obs: &[f32]) -> f32 {
        self.net.forward(obs)[0] * VALUE_SCALE
    }
}

/// Serialisable model artifact stored in the CAS.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyBundle {
    pub policy: Policy,
    pub value: Option<ValueNet>,
    pub obs_dim: usize,
    #[serde(default)]
    pub meta: serde_json::Value,
}

impl PolicyBundle {
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serializable")
    }

    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<PolicyBundle> {
        let b: PolicyBundle = serde_json::from_slice(bytes)?;
        anyhow::ensure!(
            b.obs_dim == OBS_DIM,
            "bundle obs_dim {} != current OBS_DIM {}",
            b.obs_dim,
            OBS_DIM
        );
        Ok(b)
    }
}

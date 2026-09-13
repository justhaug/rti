use serde::{Deserialize, Serialize};

/// A single tick of driver input. Mirrors the TM2020 input model:
/// analog steering in [-1, 1] (negative = left), gas and brake as booleans.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Action {
    pub steer: f32,
    pub gas: bool,
    pub brake: bool,
}

impl Action {
    pub const fn new(steer: f32, gas: bool, brake: bool) -> Self {
        Action { steer, gas, brake }
    }

    pub const fn coast() -> Self {
        Action {
            steer: 0.0,
            gas: false,
            brake: false,
        }
    }

    pub const fn full_gas() -> Self {
        Action {
            steer: 0.0,
            gas: true,
            brake: false,
        }
    }

    pub fn clamped(mut self) -> Self {
        if !self.steer.is_finite() {
            self.steer = 0.0;
        }
        self.steer = self.steer.clamp(-1.0, 1.0);
        self
    }

    /// Dense encoding used by neural nets and CAS files: [steer, gas, brake].
    pub fn to_vec(self) -> [f32; 3] {
        [self.steer, self.gas as u8 as f32, self.brake as u8 as f32]
    }

    /// Compact discrete action set used by tree/beam search. Steering is
    /// quantized to `steer_levels` values in [-1, 1].
    pub fn discrete_set(steer_levels: usize) -> Vec<Action> {
        let n = steer_levels.max(1);
        let mut out = Vec::with_capacity(n * 3);
        for i in 0..n {
            let steer = if n == 1 {
                0.0
            } else {
                -1.0 + 2.0 * i as f32 / (n - 1) as f32
            };
            out.push(Action::new(steer, true, false));
            out.push(Action::new(steer, false, false));
            out.push(Action::new(steer, false, true));
        }
        out
    }
}

/// Compact representation of an input sequence as runs of identical actions,
/// which is how TAS tools and humans think about inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionRun {
    pub action: Action,
    pub ticks: u32,
}

pub fn compress_actions(actions: &[Action]) -> Vec<ActionRun> {
    let mut runs: Vec<ActionRun> = Vec::new();
    for &a in actions {
        match runs.last_mut() {
            Some(r) if r.action == a => r.ticks += 1,
            _ => runs.push(ActionRun {
                action: a,
                ticks: 1,
            }),
        }
    }
    runs
}

pub fn expand_runs(runs: &[ActionRun]) -> Vec<Action> {
    let mut out = Vec::new();
    for r in runs {
        for _ in 0..r.ticks {
            out.push(r.action);
        }
    }
    out
}

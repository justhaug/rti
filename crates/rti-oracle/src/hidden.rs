use rand::{Rng, SeedableRng};
use rti_core::{Action, PhysicsParams, Track};
use rti_sim::{rollout, Ext, Sim, TrackGeom};

use crate::{Oracle, OracleRun};

/// Truth-world stand-in: perturbed parameters plus unmodelled effects.
/// The seed fixes the "laws of physics"; RTI never reads them directly.
#[derive(Clone, Debug)]
pub struct HiddenSim {
    params: PhysicsParams,
    ext: Ext,
    seed: u64,
}

impl HiddenSim {
    pub fn from_seed(seed: u64) -> HiddenSim {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let params =
            PhysicsParams::default().perturbed(0.12, &mut || rng.random_range(-1.0f32..1.0));
        let ext = Ext {
            downforce: rng.random_range(0.002..0.006),
            steer_lag: rng.random_range(0.02..0.08),
            gear_dip: rng.random_range(0.15..0.35),
            gear_speed: rng.random_range(28.0..42.0),
            yaw_grip_loss: rng.random_range(0.05..0.15),
        };
        HiddenSim { params, ext, seed }
    }

    /// Exposed for tests only; the research loop must not call this.
    #[doc(hidden)]
    pub fn truth(&self) -> (&PhysicsParams, &Ext) {
        (&self.params, &self.ext)
    }
}

impl Oracle for HiddenSim {
    fn name(&self) -> String {
        format!("hidden_sim#{}", self.seed)
    }
    fn available(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn run(&self, track: &Track, actions: &[Action], max_ticks: u32) -> anyhow::Result<OracleRun> {
        let sim = Sim::new(self.params.clone(), TrackGeom::new(track.clone())).with_ext(self.ext);
        let ro = rollout(&sim, &sim.initial_state(), actions, max_ticks, true);
        Ok(OracleRun {
            states: ro.states,
            result: ro.result,
            ticks: ro.ticks,
        })
    }
    fn ms_per_tick(&self) -> f64 {
        0.0002
    }
}

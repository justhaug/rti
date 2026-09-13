use serde::{Deserialize, Serialize};

use crate::ContentHash;

/// Every tunable constant of the replicated physics. The calibration loop
/// treats this as a flat vector (`to_vec`/`from_vec`) so that any search
/// method can fit it to oracle data. Add fields at the end and bump
/// `PhysicsParams::VERSION` when the semantics of the model change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsParams {
    /// Peak engine acceleration at low speed (m/s^2).
    pub engine_accel: f32,
    /// Speed (m/s) at which engine acceleration has halved.
    pub engine_falloff_speed: f32,
    /// Braking deceleration (m/s^2).
    pub brake_decel: f32,
    /// Quadratic aerodynamic drag coefficient (1/m).
    pub drag: f32,
    /// Rolling resistance deceleration (m/s^2).
    pub rolling: f32,
    /// Maximum lateral acceleration on asphalt (m/s^2).
    pub grip: f32,
    /// Grip multiplier per surface [asphalt, dirt, grass, ice].
    pub surface_grip: [f32; 4],
    /// Longitudinal (engine/brake) multiplier per surface.
    pub surface_drive: [f32; 4],
    /// Max steering angle (radians) at standstill.
    pub steer_max: f32,
    /// Steering angle shrinks with speed: steer_max / (1 + speed / steer_speed_falloff).
    pub steer_speed_falloff: f32,
    /// Steering column slew rate (full-lock per second).
    pub steer_rate: f32,
    /// Wheelbase (meters) for the kinematic bicycle model.
    pub wheelbase: f32,
    /// Fraction of lateral velocity retained per tick when sliding (0..1).
    pub slide_damping: f32,
    /// Speed above which the friction circle starts saturating (m/s).
    pub slip_onset: f32,
    /// Off-track grip multiplier.
    pub offtrack_grip: f32,
    /// Off-track drag multiplier.
    pub offtrack_drag: f32,
    /// Fraction of velocity kept after a wall hit.
    pub wall_restitution: f32,
    /// Hard speed cap (m/s).
    pub max_speed: f32,
    /// Drive (gear) engagement time constant in seconds.
    pub drive_tau: f32,
    /// Aerodynamic downforce: extra lateral grip proportional to speed² (1/m).
    #[serde(default)]
    pub downforce: f32,
    /// Fraction of horizontal speed kept when landing from a jump.
    #[serde(default = "default_landing_keep")]
    pub landing_keep: f32,
}

fn default_landing_keep() -> f32 {
    0.97
}

impl Default for PhysicsParams {
    /// Defaults fitted to human replays (finish and checkpoint times of TMX
    /// replays replayed in this simulator on compiled maps); see
    /// `crates/rti-maps/examples/replayfit.rs`. Still a first approximation.
    fn default() -> Self {
        PhysicsParams {
            engine_accel: 19.8,
            engine_falloff_speed: 55.5,
            brake_decel: 44.5,
            drag: 0.0009,
            rolling: 1.1,
            grip: 23.8,
            surface_grip: [1.67, 1.07, 0.43, 0.093],
            surface_drive: [2.54, 1.07, 0.81, 0.62],
            steer_max: 0.49,
            steer_speed_falloff: 17.7,
            steer_rate: 10.4,
            wheelbase: 2.41,
            slide_damping: 0.98,
            slip_onset: 32.4,
            offtrack_grip: 0.77,
            offtrack_drag: 7.2,
            wall_restitution: 0.8,
            max_speed: 107.0,
            drive_tau: 0.31,
            downforce: 0.0,
            landing_keep: 0.97,
        }
    }
}

impl PhysicsParams {
    pub const VERSION: u32 = 2;

    pub fn hash(&self) -> ContentHash {
        ContentHash::of_json(self)
    }

    /// Names of the flat parameter vector, in `to_vec` order.
    pub fn names() -> Vec<&'static str> {
        vec![
            "engine_accel",
            "engine_falloff_speed",
            "brake_decel",
            "drag",
            "rolling",
            "grip",
            "surface_grip.asphalt",
            "surface_grip.dirt",
            "surface_grip.grass",
            "surface_grip.ice",
            "surface_drive.asphalt",
            "surface_drive.dirt",
            "surface_drive.grass",
            "surface_drive.ice",
            "steer_max",
            "steer_speed_falloff",
            "steer_rate",
            "wheelbase",
            "slide_damping",
            "slip_onset",
            "offtrack_grip",
            "offtrack_drag",
            "wall_restitution",
            "max_speed",
            "drive_tau",
            "downforce",
            "landing_keep",
        ]
    }

    pub fn to_vec(&self) -> Vec<f32> {
        let mut v = vec![
            self.engine_accel,
            self.engine_falloff_speed,
            self.brake_decel,
            self.drag,
            self.rolling,
            self.grip,
        ];
        v.extend_from_slice(&self.surface_grip);
        v.extend_from_slice(&self.surface_drive);
        v.extend_from_slice(&[
            self.steer_max,
            self.steer_speed_falloff,
            self.steer_rate,
            self.wheelbase,
            self.slide_damping,
            self.slip_onset,
            self.offtrack_grip,
            self.offtrack_drag,
            self.wall_restitution,
            self.max_speed,
            self.drive_tau,
            self.downforce,
            self.landing_keep,
        ]);
        v
    }

    pub fn from_vec(v: &[f32]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (25..=27).contains(&v.len()),
            "expected 25-27 params, got {}",
            v.len()
        );
        Ok(PhysicsParams {
            engine_accel: v[0],
            engine_falloff_speed: v[1],
            brake_decel: v[2],
            drag: v[3],
            rolling: v[4],
            grip: v[5],
            surface_grip: [v[6], v[7], v[8], v[9]],
            surface_drive: [v[10], v[11], v[12], v[13]],
            steer_max: v[14],
            steer_speed_falloff: v[15],
            steer_rate: v[16],
            wheelbase: v[17],
            slide_damping: v[18].clamp(0.0, 1.0),
            slip_onset: v[19],
            offtrack_grip: v[20],
            offtrack_drag: v[21],
            wall_restitution: v[22].clamp(0.0, 1.0),
            max_speed: v[23],
            drive_tau: v[24].max(0.01),
            downforce: v.get(25).copied().unwrap_or(0.0).max(0.0),
            landing_keep: v.get(26).copied().unwrap_or(0.97).clamp(0.5, 1.0),
        })
    }

    /// Multiplicative perturbation, used to build calibration populations and
    /// the hidden "truth" oracle.
    pub fn perturbed(&self, scale: f32, rng: &mut impl FnMut() -> f32) -> Self {
        let v: Vec<f32> = self
            .to_vec()
            .iter()
            .map(|x| x * (1.0 + scale * rng()))
            .collect();
        Self::from_vec(&v).expect("same length")
    }
}

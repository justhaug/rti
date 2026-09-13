use serde::{Deserialize, Serialize};

/// Weights for the research value function
/// V = race improvement + information gain + novel behaviour + sim accuracy + research efficiency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ValueWeights {
    pub race_improvement: f64,
    pub information_gain: f64,
    pub novelty: f64,
    pub sim_accuracy: f64,
    pub efficiency: f64,
}

impl Default for ValueWeights {
    fn default() -> Self {
        ValueWeights {
            race_improvement: 1.0,
            information_gain: 1.0,
            novelty: 0.5,
            sim_accuracy: 1.0,
            efficiency: 0.5,
        }
    }
}

/// Per-experiment value decomposition. Each component is normalised to
/// roughly [0, 1] by the analyzer; `total` applies weights.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ValueScore {
    /// Verified (or sim) milliseconds gained on the track's best, scaled.
    pub race_improvement: f64,
    /// How much the result changed our beliefs (surprise vs. prediction).
    pub information_gain: f64,
    /// Distance of the behaviour from archived behaviours.
    pub novelty: f64,
    /// Reduction in sim/oracle divergence.
    pub sim_accuracy: f64,
    /// Value per unit cost relative to the archive's running average.
    pub efficiency: f64,
}

impl ValueScore {
    pub fn total(&self, w: &ValueWeights) -> f64 {
        self.race_improvement * w.race_improvement
            + self.information_gain * w.information_gain
            + self.novelty * w.novelty
            + self.sim_accuracy * w.sim_accuracy
            + self.efficiency * w.efficiency
    }
}

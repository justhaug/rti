use rti_core::ValueScore;

/// Normalise a millisecond improvement on a best time into [0, 1].
pub fn race_component(improvement_ms: f64, verified: bool) -> f64 {
    let x = (improvement_ms.max(0.0) / 500.0).min(1.0); // 500 ms ≈ saturating
    if verified {
        x
    } else {
        0.5 * x
    }
}

/// Reduction in divergence: (before - after) / before, clipped.
pub fn sim_accuracy_component(before: Option<f32>, after: Option<f32>) -> f64 {
    match (before, after) {
        (Some(b), Some(a)) if b > 0.0 => (((b - a) / b) as f64).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Efficiency: value per cost relative to a reference cost (Mticks + $).
pub fn efficiency_component(value_sans_eff: f64, cost_mticks: f64, cost_usd: f64) -> f64 {
    let cost_units = cost_mticks / 100.0 + cost_usd / 0.05 + 0.05;
    (value_sans_eff / cost_units).clamp(0.0, 1.0)
}

pub fn assemble(
    race: f64,
    info: f64,
    novelty: f64,
    sim_acc: f64,
    cost_mticks: f64,
    cost_usd: f64,
) -> ValueScore {
    let partial = race + info + novelty + sim_acc;
    ValueScore {
        race_improvement: race,
        information_gain: info,
        novelty,
        sim_accuracy: sim_acc,
        efficiency: efficiency_component(partial, cost_mticks, cost_usd),
    }
}

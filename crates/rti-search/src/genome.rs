use rti_core::Action;

/// Control-point genome: `steer[k]`, `drive[k]` interleaved as
/// `[s0, d0, s1, d1, ...]`, all in [-1, 1].
#[derive(Clone, Debug, PartialEq)]
pub struct Genome(pub Vec<f32>);

impl Genome {
    pub fn control_points(&self) -> usize {
        self.0.len() / 2
    }

    /// Default genome: straight, gas on (drive 0.5 so that braking is within
    /// reach of the initial exploration noise).
    pub fn straight(k: usize) -> Genome {
        let mut v = Vec::with_capacity(2 * k);
        for _ in 0..k {
            v.push(0.0);
            v.push(0.5);
        }
        Genome(v)
    }

    pub fn clamp(&mut self) {
        for x in &mut self.0 {
            if !x.is_finite() {
                *x = 0.0;
            }
            *x = x.clamp(-1.0, 1.0);
        }
    }
}

/// Drive value below which the decoder brakes; between `BRAKE_BELOW` and 0
/// the car coasts, above 0 it accelerates.
pub const BRAKE_BELOW: f32 = -0.2;

/// Action for a car at centerline distance `progress` on a track of length
/// `track_len`. Control points are spaced evenly along the track (a racing
/// line is a function of position, not time). Steering is linearly
/// interpolated between control points; drive is piecewise constant.
#[inline]
pub fn action_at(g: &Genome, track_len: f32, progress: f32) -> Action {
    let k = g.control_points().max(1);
    let pos = (progress / track_len.max(1.0)).clamp(0.0, 1.0) * (k - 1).max(1) as f32;
    let i = (pos.floor() as usize).min(k - 1);
    let j = (i + 1).min(k - 1);
    let f = pos - i as f32;
    let steer = g.0[2 * i] * (1.0 - f) + g.0[2 * j] * f;
    let drive = g.0[2 * i + 1];
    Action {
        steer: steer.clamp(-1.0, 1.0),
        gas: drive > 0.0,
        brake: drive < BRAKE_BELOW,
    }
}

/// Materialise the per-tick action sequence a genome produces on `sim`
/// (requires simulation because control points are distance-indexed).
pub fn decode(g: &Genome, sim: &rti_sim::Sim, max_ticks: u32) -> (Vec<Action>, rti_sim::Rollout) {
    let acts = std::cell::RefCell::new(Vec::with_capacity(2048));
    let len = sim.geom.total_len;
    let ro = rti_sim::rollout_with(
        sim,
        &sim.initial_state(),
        max_ticks,
        max_ticks,
        false,
        |_, s| {
            let a = action_at(g, len, s.progress);
            acts.borrow_mut().push(a);
            a
        },
    );
    (acts.into_inner(), ro)
}

/// Inverse of `decode` (lossy): fit a genome of `k` control points to an
/// action sequence by simulating it to recover where on the track each
/// action was applied. Used to warm-start continuous methods.
pub fn encode(actions: &[Action], sim: &rti_sim::Sim, k: usize) -> Genome {
    let k = k.max(2);
    let ro = rti_sim::rollout(
        sim,
        &sim.initial_state(),
        actions,
        actions.len() as u32,
        true,
    );
    let len = sim.geom.total_len.max(1.0);
    // bucket actions by control-point index of the state they were applied in
    let mut buckets: Vec<Vec<Action>> = vec![Vec::new(); k];
    for (i, a) in actions
        .iter()
        .enumerate()
        .take(ro.states.len().saturating_sub(1))
    {
        let pos = (ro.states[i].progress / len).clamp(0.0, 1.0) * (k - 1) as f32;
        buckets[(pos.round() as usize).min(k - 1)].push(*a);
    }
    let mut v = Vec::with_capacity(2 * k);
    let mut last = (0.0f32, 0.5f32);
    for bucket in &buckets {
        if bucket.is_empty() {
            v.push(last.0);
            v.push(last.1);
            continue;
        }
        let slice = bucket.as_slice();
        let steer = slice.iter().map(|a| a.steer).sum::<f32>() / slice.len() as f32;
        let gas = slice.iter().filter(|a| a.gas).count() as f32 / slice.len() as f32;
        let brake = slice.iter().filter(|a| a.brake).count() as f32 / slice.len() as f32;
        let drive = if brake > 0.5 {
            -0.6
        } else if gas > 0.5 {
            0.5
        } else {
            -0.1
        };
        v.push(steer);
        v.push(drive);
        last = (steer, drive);
    }
    Genome(v)
}

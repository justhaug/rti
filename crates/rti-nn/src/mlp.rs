use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Tanh,
    Relu,
}

impl Activation {
    #[inline]
    fn f(self, x: f32) -> f32 {
        match self {
            Activation::Tanh => x.tanh(),
            Activation::Relu => x.max(0.0),
        }
    }
    /// Derivative expressed in terms of the activated output `y`.
    #[inline]
    fn df(self, y: f32) -> f32 {
        match self {
            Activation::Tanh => 1.0 - y * y,
            Activation::Relu => {
                if y > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Loss functions supported by the trainer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Loss {
    /// Mean squared error over all outputs.
    Mse,
    /// Output layout `[steer, gas_logit, brake_logit]`: MSE on `tanh(steer)`
    /// against the target steer plus BCE-with-logits on gas/brake.
    PolicyLoss,
}

/// Fully-connected network. Hidden layers use `activation`; the output layer
/// is linear. Weights are row-major `out × in` per layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mlp {
    pub sizes: Vec<usize>,
    pub weights: Vec<Vec<f32>>,
    pub biases: Vec<Vec<f32>>,
    pub activation: Activation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrainConfig {
    pub epochs: usize,
    pub lr: f32,
    pub batch_size: usize,
    pub seed: u64,
    /// Decoupled weight decay (AdamW style), applied per step.
    pub weight_decay: f32,
    /// L2 penalty added to the gradient.
    pub l2: f32,
}

impl Default for TrainConfig {
    fn default() -> Self {
        TrainConfig {
            epochs: 30,
            lr: 1e-3,
            batch_size: 128,
            seed: 0,
            weight_decay: 0.0,
            l2: 0.0,
        }
    }
}

/// Gradient accumulator with the same shape as the parameters.
#[derive(Clone)]
struct Grads {
    w: Vec<Vec<f32>>,
    b: Vec<Vec<f32>>,
}

impl Grads {
    fn zeros(net: &Mlp) -> Grads {
        Grads {
            w: net.weights.iter().map(|w| vec![0.0; w.len()]).collect(),
            b: net.biases.iter().map(|b| vec![0.0; b.len()]).collect(),
        }
    }
    fn add(&mut self, o: &Grads) {
        for (a, b) in self.w.iter_mut().zip(&o.w) {
            for (x, y) in a.iter_mut().zip(b) {
                *x += *y;
            }
        }
        for (a, b) in self.b.iter_mut().zip(&o.b) {
            for (x, y) in a.iter_mut().zip(b) {
                *x += *y;
            }
        }
    }
}

#[inline]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Numerically stable BCE with logits.
#[inline]
fn bce_logits(z: f32, t: f32) -> f32 {
    z.max(0.0) - z * t + (1.0 + (-z.abs()).exp()).ln()
}

impl Mlp {
    /// Xavier-initialised network with the given layer sizes.
    pub fn new(sizes: &[usize], activation: Activation, seed: u64) -> Mlp {
        assert!(sizes.len() >= 2, "need at least input and output sizes");
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let mut weights = Vec::new();
        let mut biases = Vec::new();
        for l in 0..sizes.len() - 1 {
            let (n_in, n_out) = (sizes[l], sizes[l + 1]);
            let limit = (6.0 / (n_in + n_out) as f32).sqrt();
            let w: Vec<f32> = (0..n_in * n_out)
                .map(|_| rng.random_range(-limit..limit))
                .collect();
            weights.push(w);
            biases.push(vec![0.0; n_out]);
        }
        Mlp {
            sizes: sizes.to_vec(),
            weights,
            biases,
            activation,
        }
    }

    pub fn n_layers(&self) -> usize {
        self.weights.len()
    }

    pub fn input_dim(&self) -> usize {
        self.sizes[0]
    }

    pub fn output_dim(&self) -> usize {
        *self.sizes.last().unwrap()
    }

    pub fn n_params(&self) -> usize {
        self.weights.iter().map(|w| w.len()).sum::<usize>()
            + self.biases.iter().map(|b| b.len()).sum::<usize>()
    }

    /// Forward pass returning every layer's activations (index 0 = input).
    fn forward_all(&self, x: &[f32]) -> Vec<Vec<f32>> {
        debug_assert_eq!(x.len(), self.sizes[0]);
        let mut acts = Vec::with_capacity(self.sizes.len());
        acts.push(x.to_vec());
        let last = self.n_layers() - 1;
        for l in 0..self.n_layers() {
            let (n_in, n_out) = (self.sizes[l], self.sizes[l + 1]);
            let w = &self.weights[l];
            let b = &self.biases[l];
            let inp = &acts[l];
            let mut out = Vec::with_capacity(n_out);
            for o in 0..n_out {
                let row = &w[o * n_in..(o + 1) * n_in];
                let mut s = b[o];
                for i in 0..n_in {
                    s += row[i] * inp[i];
                }
                out.push(if l == last { s } else { self.activation.f(s) });
            }
            acts.push(out);
        }
        acts
    }

    pub fn forward(&self, x: &[f32]) -> Vec<f32> {
        self.forward_all(x).pop().unwrap()
    }

    pub fn forward_batch(&self, xs: &[Vec<f32>]) -> Vec<Vec<f32>> {
        xs.par_iter().map(|x| self.forward(x)).collect()
    }

    /// Loss and gradient of the loss w.r.t. the network output for one sample.
    fn loss_and_dout(&self, out: &[f32], y: &[f32], loss: Loss) -> (f32, Vec<f32>) {
        match loss {
            Loss::Mse => {
                let n = out.len() as f32;
                let mut l = 0.0;
                let d: Vec<f32> = out
                    .iter()
                    .zip(y)
                    .map(|(o, t)| {
                        let e = o - t;
                        l += e * e;
                        2.0 * e / n
                    })
                    .collect();
                (l / n, d)
            }
            Loss::PolicyLoss => {
                debug_assert!(out.len() >= 3 && y.len() >= 3);
                let s = out[0].tanh();
                let es = s - y[0];
                let l_steer = es * es;
                let d_steer = 2.0 * es * (1.0 - s * s);
                let l_gas = bce_logits(out[1], y[1]);
                let d_gas = sigmoid(out[1]) - y[1];
                let l_brake = bce_logits(out[2], y[2]);
                let d_brake = sigmoid(out[2]) - y[2];
                let mut d = vec![0.0; out.len()];
                d[0] = d_steer;
                d[1] = d_gas;
                d[2] = d_brake;
                (l_steer + l_gas + l_brake, d)
            }
        }
    }

    /// Loss of the network on a dataset (no gradient).
    pub fn evaluate(&self, xs: &[Vec<f32>], ys: &[Vec<f32>], loss: Loss) -> f32 {
        if xs.is_empty() {
            return 0.0;
        }
        let total: f64 = xs
            .par_iter()
            .zip(ys)
            .map(|(x, y)| self.loss_and_dout(&self.forward(x), y, loss).0 as f64)
            .sum();
        (total / xs.len() as f64) as f32
    }

    /// Backprop for one sample; returns (loss, grads).
    fn backprop(&self, x: &[f32], y: &[f32], loss: Loss) -> (f32, Grads) {
        let acts = self.forward_all(x);
        let (l, mut delta) = self.loss_and_dout(acts.last().unwrap(), y, loss);
        let mut g = Grads::zeros(self);
        for layer in (0..self.n_layers()).rev() {
            let (n_in, n_out) = (self.sizes[layer], self.sizes[layer + 1]);
            let inp = &acts[layer];
            let gw = &mut g.w[layer];
            let gb = &mut g.b[layer];
            for o in 0..n_out {
                let d = delta[o];
                gb[o] += d;
                let row = &mut gw[o * n_in..(o + 1) * n_in];
                for i in 0..n_in {
                    row[i] += d * inp[i];
                }
            }
            if layer > 0 {
                let w = &self.weights[layer];
                let mut prev = vec![0.0f32; n_in];
                for o in 0..n_out {
                    let d = delta[o];
                    let row = &w[o * n_in..(o + 1) * n_in];
                    for i in 0..n_in {
                        prev[i] += d * row[i];
                    }
                }
                for i in 0..n_in {
                    prev[i] *= self.activation.df(inp[i]);
                }
                delta = prev;
            }
        }
        (l, g)
    }

    /// Mean loss and mean gradient over a batch, computed in parallel.
    fn batch_grads(&self, xs: &[&Vec<f32>], ys: &[&Vec<f32>], loss: Loss) -> (f32, Grads) {
        let n = xs.len().max(1) as f32;
        let (l, mut g) = xs
            .par_iter()
            .zip(ys.par_iter())
            .map(|(x, y)| self.backprop(x, y, loss))
            .reduce(
                || (0.0f32, Grads::zeros(self)),
                |mut a, b| {
                    a.0 += b.0;
                    a.1.add(&b.1);
                    a
                },
            );
        for w in g.w.iter_mut().chain(g.b.iter_mut()) {
            for v in w.iter_mut() {
                *v /= n;
            }
        }
        (l / n, g)
    }

    /// Minibatch Adam training. Returns the mean training loss per epoch.
    pub fn train(
        &mut self,
        xs: &[Vec<f32>],
        ys: &[Vec<f32>],
        loss: Loss,
        cfg: &TrainConfig,
    ) -> Vec<f32> {
        assert_eq!(xs.len(), ys.len());
        if xs.is_empty() {
            return vec![];
        }
        let mut rng = rand::rngs::StdRng::seed_from_u64(cfg.seed);
        let mut m = Grads::zeros(self);
        let mut v = Grads::zeros(self);
        let (b1, b2, eps) = (0.9f32, 0.999f32, 1e-8f32);
        let mut step = 0u32;
        let bs = cfg.batch_size.max(1);
        let mut idx: Vec<usize> = (0..xs.len()).collect();
        let mut history = Vec::with_capacity(cfg.epochs);
        for _ in 0..cfg.epochs {
            // Fisher-Yates shuffle with the seeded rng.
            for i in (1..idx.len()).rev() {
                let j = rng.random_range(0..=i);
                idx.swap(i, j);
            }
            let mut epoch_loss = 0.0f64;
            let mut n_batches = 0usize;
            for chunk in idx.chunks(bs) {
                let bx: Vec<&Vec<f32>> = chunk.iter().map(|&i| &xs[i]).collect();
                let by: Vec<&Vec<f32>> = chunk.iter().map(|&i| &ys[i]).collect();
                let (l, g) = self.batch_grads(&bx, &by, loss);
                epoch_loss += l as f64;
                n_batches += 1;
                step += 1;
                let bc1 = 1.0 - b1.powi(step as i32);
                let bc2 = 1.0 - b2.powi(step as i32);
                for layer in 0..self.n_layers() {
                    adam_update(
                        &mut self.weights[layer],
                        &g.w[layer],
                        &mut m.w[layer],
                        &mut v.w[layer],
                        cfg,
                        b1,
                        b2,
                        eps,
                        bc1,
                        bc2,
                        true,
                    );
                    adam_update(
                        &mut self.biases[layer],
                        &g.b[layer],
                        &mut m.b[layer],
                        &mut v.b[layer],
                        cfg,
                        b1,
                        b2,
                        eps,
                        bc1,
                        bc2,
                        false,
                    );
                }
            }
            history.push((epoch_loss / n_batches.max(1) as f64) as f32);
        }
        history
    }

    /// Finite-difference gradient of the mean loss over a batch, for tests.
    #[doc(hidden)]
    pub fn numerical_grad(
        &self,
        xs: &[Vec<f32>],
        ys: &[Vec<f32>],
        loss: Loss,
        h: f32,
    ) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
        let mut net = self.clone();
        let mut gw = Vec::new();
        let mut gb = Vec::new();
        for l in 0..self.n_layers() {
            let mut g = vec![0.0; self.weights[l].len()];
            #[allow(clippy::needless_range_loop)]
            for i in 0..g.len() {
                let orig = net.weights[l][i];
                net.weights[l][i] = orig + h;
                let lp = net.evaluate(xs, ys, loss);
                net.weights[l][i] = orig - h;
                let lm = net.evaluate(xs, ys, loss);
                net.weights[l][i] = orig;
                g[i] = (lp - lm) / (2.0 * h);
            }
            gw.push(g);
            let mut g = vec![0.0; self.biases[l].len()];
            #[allow(clippy::needless_range_loop)]
            for i in 0..g.len() {
                let orig = net.biases[l][i];
                net.biases[l][i] = orig + h;
                let lp = net.evaluate(xs, ys, loss);
                net.biases[l][i] = orig - h;
                let lm = net.evaluate(xs, ys, loss);
                net.biases[l][i] = orig;
                g[i] = (lp - lm) / (2.0 * h);
            }
            gb.push(g);
        }
        (gw, gb)
    }

    /// Analytic gradient of the mean loss over a batch, for tests.
    #[doc(hidden)]
    pub fn analytic_grad(
        &self,
        xs: &[Vec<f32>],
        ys: &[Vec<f32>],
        loss: Loss,
    ) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
        let bx: Vec<&Vec<f32>> = xs.iter().collect();
        let by: Vec<&Vec<f32>> = ys.iter().collect();
        let (_, g) = self.batch_grads(&bx, &by, loss);
        (g.w, g.b)
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn adam_update(
    p: &mut [f32],
    g: &[f32],
    m: &mut [f32],
    v: &mut [f32],
    cfg: &TrainConfig,
    b1: f32,
    b2: f32,
    eps: f32,
    bc1: f32,
    bc2: f32,
    decay: bool,
) {
    for i in 0..p.len() {
        let mut gi = g[i];
        if decay && cfg.l2 > 0.0 {
            gi += cfg.l2 * p[i];
        }
        m[i] = b1 * m[i] + (1.0 - b1) * gi;
        v[i] = b2 * v[i] + (1.0 - b2) * gi * gi;
        let mh = m[i] / bc1;
        let vh = v[i] / bc2;
        p[i] -= cfg.lr * mh / (vh.sqrt() + eps);
        if decay && cfg.weight_decay > 0.0 {
            p[i] -= cfg.lr * cfg.weight_decay * p[i];
        }
    }
}

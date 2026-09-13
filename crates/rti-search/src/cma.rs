//! (mu/mu_w, lambda)-CMA-ES with full covariance, following Hansen's
//! tutorial. Eigendecomposition by cyclic Jacobi, refreshed lazily.

use rand::Rng;
use rand_distr::{Distribution, StandardNormal};

use crate::common::{Evaluator, SearchInput, SearchOutcome};
use crate::genome::Genome;

/// Symmetric eigendecomposition via cyclic Jacobi. Returns (eigenvalues, eigenvectors as columns in row-major `n×n`).
fn jacobi_eigen(a: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut a = a.to_vec();
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _sweep in 0..50 {
        let mut off = 0.0;
        for i in 0..n {
            for j in (i + 1)..n {
                off += a[i * n + j] * a[i * n + j];
            }
        }
        if off < 1e-18 {
            break;
        }
        for p in 0..n {
            for q in (p + 1)..n {
                let apq = a[p * n + q];
                if apq.abs() < 1e-300 {
                    continue;
                }
                let app = a[p * n + p];
                let aqq = a[q * n + q];
                let theta = (aqq - app) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let akp = a[k * n + p];
                    let akq = a[k * n + q];
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let apk = a[p * n + k];
                    let aqk = a[q * n + k];
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let vkp = v[k * n + p];
                    let vkq = v[k * n + q];
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    let evals: Vec<f64> = (0..n).map(|i| a[i * n + i].max(1e-20)).collect();
    (evals, v)
}

pub fn cma_es(input: &SearchInput) -> SearchOutcome {
    // full-covariance CMA-ES is O(n^3) per eigendecomposition; cap dims
    let k = input
        .params
        .control_points
        .unwrap_or_else(|| input.control_points().min(60));
    let n = 2 * k;
    let max_ticks = input.max_ticks();
    let lambda = input
        .params
        .population
        .unwrap_or(4 + (3.0 * (n as f64).ln()).floor() as usize * 4);
    let mu = lambda / 2;
    let sigma0 = input.params.sigma.unwrap_or(0.3) as f64;
    let max_iters = input.params.iters.unwrap_or(usize::MAX);
    let mut rng = input.rng();
    let mut ev = Evaluator::new(input.sim, max_ticks, input.budget);

    // weights
    let mut w: Vec<f64> = (0..mu)
        .map(|i| ((mu as f64) + 0.5).ln() - ((i + 1) as f64).ln())
        .collect();
    let ws: f64 = w.iter().sum();
    for x in &mut w {
        *x /= ws;
    }
    let mueff = 1.0 / w.iter().map(|x| x * x).sum::<f64>();
    let nf = n as f64;
    let cc = (4.0 + mueff / nf) / (nf + 4.0 + 2.0 * mueff / nf);
    let cs = (mueff + 2.0) / (nf + mueff + 5.0);
    let c1 = 2.0 / ((nf + 1.3).powi(2) + mueff);
    let cmu = (1.0 - c1).min(2.0 * (mueff - 2.0 + 1.0 / mueff) / ((nf + 2.0).powi(2) + mueff));
    let damps = 1.0 + 2.0 * (0.0f64).max(((mueff - 1.0) / (nf + 1.0)).sqrt() - 1.0) + cs;
    let chi_n = nf.sqrt() * (1.0 - 1.0 / (4.0 * nf) + 1.0 / (21.0 * nf * nf));

    let init = input.warm_genome(k).unwrap_or_else(|| Genome::straight(k));
    let mut xmean: Vec<f64> = init.0.iter().map(|&x| x as f64).collect();
    let mut sigma = sigma0;
    let mut pc = vec![0.0; n];
    let mut ps = vec![0.0; n];
    let mut c = vec![0.0; n * n];
    for i in 0..n {
        c[i * n + i] = 1.0;
    }
    let (mut d, mut b) = (vec![1.0; n], c.clone());
    let mut eigen_age = 0usize;
    let lazy_gap = (1.0 / (c1 + cmu) / nf / 10.0).max(1.0) as usize;
    let mut gen = 0usize;

    while !ev.exhausted() && gen < max_iters {
        if eigen_age == 0 {
            let (evals, evecs) = jacobi_eigen(&c, n);
            d = evals.iter().map(|x| x.sqrt()).collect();
            b = evecs;
        }
        eigen_age = (eigen_age + 1) % lazy_gap;

        // sample
        let mut zs: Vec<Vec<f64>> = Vec::with_capacity(lambda);
        let mut xs: Vec<Vec<f64>> = Vec::with_capacity(lambda);
        for _ in 0..lambda {
            let z: Vec<f64> = (0..n)
                .map(|_| <StandardNormal as Distribution<f64>>::sample(&StandardNormal, &mut rng))
                .collect();
            // y = B * (D .* z)
            let mut y = vec![0.0; n];
            for i in 0..n {
                let mut acc = 0.0;
                for j in 0..n {
                    acc += b[i * n + j] * d[j] * z[j];
                }
                y[i] = acc;
            }
            let x: Vec<f64> = (0..n)
                .map(|i| (xmean[i] + sigma * y[i]).clamp(-1.0, 1.0))
                .collect();
            zs.push(y);
            xs.push(x);
        }
        let genomes: Vec<Genome> = xs
            .iter()
            .map(|x| Genome(x.iter().map(|&v| v as f32).collect()))
            .collect();
        let (objs, _) = ev.eval_genomes(&genomes);
        let mut idx: Vec<usize> = (0..lambda).collect();
        idx.sort_by(|&a, &bb| objs[a].partial_cmp(&objs[bb]).unwrap());

        // recombination
        let xold = xmean.clone();
        let mut ymean = vec![0.0; n];
        for (r, &i) in idx[..mu].iter().enumerate() {
            for j in 0..n {
                ymean[j] += w[r] * (xs[i][j] - xold[j]) / sigma;
            }
        }
        for j in 0..n {
            xmean[j] = xold[j] + sigma * ymean[j];
        }
        // C^-1/2 * ymean = B * D^-1 * B^T * ymean
        let mut tmp = vec![0.0; n];
        for j in 0..n {
            let mut acc = 0.0;
            for i in 0..n {
                acc += b[i * n + j] * ymean[i];
            }
            tmp[j] = acc / d[j];
        }
        let mut cinv_y = vec![0.0; n];
        for i in 0..n {
            let mut acc = 0.0;
            for j in 0..n {
                acc += b[i * n + j] * tmp[j];
            }
            cinv_y[i] = acc;
        }
        for i in 0..n {
            ps[i] = (1.0 - cs) * ps[i] + (cs * (2.0 - cs) * mueff).sqrt() * cinv_y[i];
        }
        let ps_norm = ps.iter().map(|x| x * x).sum::<f64>().sqrt();
        let hsig = ps_norm / (1.0 - (1.0 - cs).powi(2 * (gen as i32 + 1))).sqrt() / chi_n
            < 1.4 + 2.0 / (nf + 1.0);
        let hs = if hsig { 1.0 } else { 0.0 };
        for i in 0..n {
            pc[i] = (1.0 - cc) * pc[i] + hs * (cc * (2.0 - cc) * mueff).sqrt() * ymean[i];
        }
        // covariance update
        let c1a = c1 * (1.0 - (1.0 - hs) * cc * (2.0 - cc));
        for i in 0..n {
            for j in 0..n {
                let mut rank_mu = 0.0;
                for (r, &ix) in idx[..mu].iter().enumerate() {
                    let yi = (xs[ix][i] - xold[i]) / sigma;
                    let yj = (xs[ix][j] - xold[j]) / sigma;
                    rank_mu += w[r] * yi * yj;
                }
                c[i * n + j] =
                    (1.0 - c1a - cmu) * c[i * n + j] + c1 * pc[i] * pc[j] + cmu * rank_mu;
            }
        }
        sigma *= ((cs / damps) * (ps_norm / chi_n - 1.0)).exp();
        sigma = sigma.clamp(1e-3, 1.0);
        gen += 1;
    }
    let _ = rng.random::<f64>();
    ev.finish("cma_es", serde_json::json!({"generations": gen, "control_points": k, "lambda": lambda, "final_sigma": sigma}))
}

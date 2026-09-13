use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateResult {
    pub command: String,
    pub ok: bool,
    pub duration_ms: u64,
    pub output_tail: String,
}

/// Run a shell command in `dir` with a timeout; returns (ok, combined output tail).
pub fn run_shell(
    dir: &Path,
    cmd: &str,
    timeout: Duration,
    tail_bytes: usize,
) -> (bool, String, u64) {
    let started = Instant::now();
    let child = Command::new("bash")
        .arg("-lc")
        .arg(cmd)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("CARGO_TERM_COLOR", "never")
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (false, format!("spawn failed: {e}"), 0),
    };
    // Read output on threads so pipes never fill up.
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let h1 = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut b);
        b
    });
    let h2 = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut b);
        b
    });
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    timed_out = true;
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    let mut out = h1.join().unwrap_or_default();
    let err = h2.join().unwrap_or_default();
    out.extend_from_slice(b"\n");
    out.extend_from_slice(&err);
    let mut text = String::from_utf8_lossy(&out).to_string();
    if timed_out {
        text.push_str(&format!("\n[timed out after {}s]", timeout.as_secs()));
    }
    if text.len() > tail_bytes {
        let cut = text.len() - tail_bytes;
        let cut = text
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= cut)
            .unwrap_or(cut);
        text = format!("...[truncated]...\n{}", &text[cut..]);
    }
    let ok = status.map(|s| s.success()).unwrap_or(false);
    (ok, text, started.elapsed().as_millis() as u64)
}

/// Run every gate command in order, stopping at the first failure.
pub fn run_gates(dir: &Path, gates: &[String], timeout_secs: u64) -> Vec<GateResult> {
    let mut results = vec![];
    for g in gates {
        let (ok, tail, ms) = run_shell(dir, g, Duration::from_secs(timeout_secs), 4000);
        tracing::info!(gate = %g, ok, ms, "gate");
        results.push(GateResult {
            command: g.clone(),
            ok,
            duration_ms: ms,
            output_tail: tail,
        });
        if !ok {
            break;
        }
    }
    results
}

/// Parse "X Mticks/s" from the sim bench output.
pub fn parse_mticks(output: &str) -> Option<f64> {
    for line in output.lines() {
        if let Some(idx) = line.find("Mticks/s") {
            let head = &line[..idx];
            if let Some(tok) = head.split_whitespace().last().or(head.rsplit('=').next()) {
                if let Ok(v) = tok.trim().trim_start_matches('=').parse::<f64>() {
                    return Some(v);
                }
            }
            // fallback: last number before the unit
            let nums: Vec<f64> = head
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .filter_map(|s| s.parse().ok())
                .collect();
            if let Some(v) = nums.last() {
                return Some(*v);
            }
        }
    }
    None
}

/// Benchmark gate: run the sim throughput example in `dir` and compare with a
/// baseline. Returns (ok, measured Mticks/s, output).
pub fn bench_gate(
    dir: &Path,
    baseline_mticks: Option<f64>,
    max_regression: f64,
    timeout_secs: u64,
) -> (bool, Option<f64>, String) {
    let (ok, out, _) = run_shell(
        dir,
        "cargo run --release -q -p rti-sim --example bench",
        Duration::from_secs(timeout_secs),
        4000,
    );
    if !ok {
        return (false, None, out);
    }
    let measured = parse_mticks(&out);
    match (measured, baseline_mticks) {
        (Some(m), Some(b)) if b > 0.0 => (m >= b * (1.0 - max_regression), Some(m), out),
        (Some(m), _) => (true, Some(m), out),
        (None, _) => (false, None, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_bench_line() {
        assert_eq!(
            parse_mticks("8192000 ticks in 0.08s = 105.2 Mticks/s (32 threads)"),
            Some(105.2)
        );
        assert_eq!(
            parse_mticks("single-thread: 6.93 Mticks/s, result ..."),
            Some(6.93)
        );
        assert_eq!(parse_mticks("nothing"), None);
    }
}

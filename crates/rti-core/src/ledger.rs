use serde::{Deserialize, Serialize};
use std::ops::AddAssign;
use std::time::Instant;

/// The compute/accounting record attached to every experiment, LLM call and
/// coding task. RTI learns cost-effectiveness from these.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct CostRecord {
    pub sim_ticks: u64,
    pub oracle_ticks: u64,
    pub wall_ms: u64,
    pub cpu_ms: u64,
    pub llm_calls: u32,
    pub llm_prompt_tokens: u64,
    pub llm_completion_tokens: u64,
    pub llm_usd: f64,
}

impl AddAssign<&CostRecord> for CostRecord {
    fn add_assign(&mut self, o: &CostRecord) {
        self.sim_ticks += o.sim_ticks;
        self.oracle_ticks += o.oracle_ticks;
        self.wall_ms += o.wall_ms;
        self.cpu_ms += o.cpu_ms;
        self.llm_calls += o.llm_calls;
        self.llm_prompt_tokens += o.llm_prompt_tokens;
        self.llm_completion_tokens += o.llm_completion_tokens;
        self.llm_usd += o.llm_usd;
    }
}

impl CostRecord {
    pub fn mticks(&self) -> f64 {
        self.sim_ticks as f64 / 1e6
    }
    pub fn ticks_per_second(&self) -> f64 {
        if self.wall_ms == 0 {
            0.0
        } else {
            self.sim_ticks as f64 / (self.wall_ms as f64 / 1000.0)
        }
    }
}

/// Stopwatch that measures wall and process CPU time for a scoped block.
pub struct CostTimer {
    start: Instant,
    cpu_start: f64,
}

impl Default for CostTimer {
    fn default() -> Self {
        Self::start()
    }
}

impl CostTimer {
    pub fn start() -> Self {
        CostTimer {
            start: Instant::now(),
            cpu_start: process_cpu_seconds(),
        }
    }

    pub fn wall_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn cpu_ms(&self) -> u64 {
        ((process_cpu_seconds() - self.cpu_start).max(0.0) * 1000.0) as u64
    }

    pub fn finish(&self, sim_ticks: u64) -> CostRecord {
        CostRecord {
            sim_ticks,
            wall_ms: self.wall_ms(),
            cpu_ms: self.cpu_ms(),
            ..Default::default()
        }
    }
}

/// Process CPU seconds (user+sys) from /proc; 0 on non-Linux.
pub fn process_cpu_seconds() -> f64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/stat") {
            // fields after the closing paren of the comm field
            if let Some(idx) = s.rfind(')') {
                let rest: Vec<&str> = s[idx + 1..].split_whitespace().collect();
                // rest[11] = utime, rest[12] = stime (fields 14 and 15 overall)
                if rest.len() > 12 {
                    let ut: f64 = rest[11].parse().unwrap_or(0.0);
                    let st: f64 = rest[12].parse().unwrap_or(0.0);
                    let hz = 100.0; // CLK_TCK is 100 on essentially all Linux builds
                    return (ut + st) / hz;
                }
            }
        }
        0.0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}

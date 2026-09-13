//! `rti` — command-line entry point for Recursive Trackmania Intelligence.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rti_core::experiment::{SearchMethod, SearchParams};
use rti_core::{ExperimentSpec, Provenance, RtiConfig};
use rti_research::Session;

#[derive(Parser)]
#[command(name = "rti", version, about = "Recursive Trackmania Intelligence")]
struct Cli {
    /// Project root (contains rti.toml, tracks/, data/).
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a default rti.toml, tracks/ and initialise the archive.
    Init,
    /// Show archive status, ledger and configuration.
    Status,
    /// Print the researcher's context summary (what the brain reads).
    Context,
    /// Track utilities.
    Track {
        #[command(subcommand)]
        cmd: TrackCmd,
    },
    /// Simulator throughput benchmark.
    Bench {
        #[arg(long, default_value_t = 20_000_000)]
        ticks: u64,
    },
    /// Run one search method on a track (archived as an experiment).
    Search {
        track: String,
        #[arg(long, default_value = "beam")]
        method: String,
        #[arg(long, default_value_t = 20_000_000)]
        ticks: u64,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Warm-start trajectory hash.
        #[arg(long)]
        warm: Option<String>,
        /// Extra params as JSON, e.g. '{"population":256}'.
        #[arg(long)]
        params: Option<String>,
    },
    /// Replay a trajectory in the oracle and record the divergence.
    Verify { trajectory: String },
    /// Fit physics parameters to oracle probes.
    Calibrate {
        #[arg(long)]
        track: Vec<String>,
        #[arg(long, default_value_t = 40_000_000)]
        ticks: u64,
        #[arg(long, default_value_t = 8)]
        probes: usize,
    },
    /// Train a policy by behaviour cloning from archived bests.
    Train {
        #[arg(long)]
        track: Vec<String>,
        #[arg(long, default_value_t = 30)]
        epochs: usize,
        #[arg(long, default_value_t = 64)]
        hidden: usize,
    },
    /// Run an ExperimentSpec given as JSON (string or @file).
    Experiment { spec: String },
    /// Run the research loop.
    Research {
        /// Number of cycles (0 = until budget exhausted).
        #[arg(long, default_value_t = 1)]
        cycles: u32,
        /// Force the scripted brain (no LLM).
        #[arg(long)]
        scripted: bool,
        /// Also execute queued tasks after each cycle.
        #[arg(long)]
        with_tasks: bool,
    },
    /// Task queue: list or execute pending tasks (verify/calibrate/coding).
    Tasks {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// Run a coding task through the agent harness right now.
    Code {
        title: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        no_bench_guard: bool,
    },
    /// Query the archive with SQL (SELECT only).
    Query { sql: String },
    /// Show ledger totals by category.
    Ledger,
    /// Talk to a role once (debugging the LLM setup).
    Chat {
        #[arg(long, default_value = "researcher")]
        role: String,
        prompt: String,
    },
    /// Check oracle connectivity.
    Oracle,
    /// Export a trajectory (inputs as TAS-style runs) to stdout.
    Export { trajectory: String },
    /// Real TM2020 maps: search Trackmania Exchange, import into the simulator.
    Map {
        #[command(subcommand)]
        cmd: MapCmd,
    },
    /// Run the HTTP server: JSON API, operator chat and UI.
    Serve {
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
}

#[derive(Subcommand)]
enum TrackCmd {
    List,
    Show {
        name: String,
    },
    /// Generate a procedural track and register it.
    Generate {
        name: String,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value_t = 8)]
        segments: usize,
    },
}

#[derive(Subcommand)]
enum MapCmd {
    /// Search Trackmania Exchange.
    Search {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        author: Option<String>,
        /// Comma-separated TMX tag ids (3=Tech, 25=Mini, 14=Ice, 15=Dirt, 2=FullSpeed).
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value_t = 10)]
        count: u32,
    },
    /// Download/parse/compile a map (TMX id, URL or .Map.Gbx path) and register it as a track.
    Import {
        source: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Parse a .Map.Gbx and print what was decoded (no archive changes).
    Inspect { file: PathBuf },
    /// List imported maps.
    List,
}

#[derive(Subcommand)]
enum TaskCmd {
    List,
    /// Execute up to N pending tasks.
    Run {
        #[arg(long, default_value_t = 1)]
        n: u32,
    },
    /// Queue a coding task.
    Add {
        title: String,
        #[arg(long)]
        description: String,
        #[arg(long, default_value_t = 2)]
        priority: i32,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ureq=warn".into()),
        )
        .with_target(false)
        .compact()
        .init();
    let cli = Cli::parse();
    let root = std::fs::canonicalize(&cli.root).unwrap_or(cli.root.clone());
    match cli.cmd {
        Cmd::Init => {
            let cfg_path = root.join("rti.toml");
            if !cfg_path.exists() {
                std::fs::write(&cfg_path, RtiConfig::default().to_toml())?;
                println!("wrote {}", cfg_path.display());
            }
            let s = Session::open(&root)?;
            let n = s.sync_tracks()?;
            let (h, _) = s.archive.current_physics()?;
            println!(
                "archive at {} ({} tracks, physics {})",
                s.cfg.data_dir.display(),
                n,
                h.short()
            );
            println!(
                "LLM: {}",
                if s.llm.is_configured() {
                    "configured"
                } else {
                    "no API key (scripted brain will be used)"
                }
            );
        }
        Cmd::Status => {
            let s = Session::open(&root)?;
            println!("{}", serde_json::to_string_pretty(&s.archive.status()?)?);
            let t = s.archive.ledger_totals()?;
            println!(
                "ledger: {:.1} Mticks sim, {} oracle ticks, {:.1} s wall, {} LLM calls, ${:.4}",
                t.mticks(),
                t.oracle_ticks,
                t.wall_ms as f64 / 1000.0,
                t.llm_calls,
                t.llm_usd
            );
            println!("roles: {:?}", s.cfg.llm.roles);
        }
        Cmd::Context => {
            let s = Session::open(&root)?;
            println!("{}", rti_research::context::build_context(&s)?);
        }
        Cmd::Track { cmd } => {
            let s = Session::open(&root)?;
            match cmd {
                TrackCmd::List => {
                    for t in s.archive.tracks()? {
                        let bs = s.archive.best_time(&t.name, "sim")?;
                        let bo = s.archive.best_time(&t.name, "oracle")?;
                        println!(
                            "{:<16} {:>7.0} m {:>4} nodes  sim best {:>8}  verified {:>8}",
                            t.name,
                            t.length_m,
                            t.n_nodes,
                            bs.map(|b| b.1.to_string()).unwrap_or("-".into()),
                            bo.map(|b| b.1.to_string()).unwrap_or("-".into())
                        );
                    }
                }
                TrackCmd::Show { name } => {
                    let t = s.track(&name)?;
                    println!("{}", serde_json::to_string_pretty(&t)?);
                }
                TrackCmd::Generate {
                    name,
                    seed,
                    segments,
                } => {
                    let r = run_spec(
                        &s,
                        ExperimentSpec::GenerateTrack {
                            name,
                            seed,
                            segments,
                            half_width: 8.0,
                        },
                    )?;
                    println!("{}", r.summary);
                }
            }
        }
        Cmd::Bench { ticks } => {
            let s = Session::open(&root)?;
            let r = run_spec(&s, ExperimentSpec::Benchmark { ticks })?;
            println!("{}", r.summary);
        }
        Cmd::Search {
            track,
            method,
            ticks,
            seed,
            warm,
            params,
        } => {
            let s = Session::open(&root)?;
            let method = SearchMethod::parse(&method).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown method {method}; one of {:?}",
                    SearchMethod::ALL
                        .iter()
                        .map(|m| m.name())
                        .collect::<Vec<_>>()
                )
            })?;
            let params: SearchParams = match params {
                Some(p) => serde_json::from_str(&p)?,
                None => SearchParams::default(),
            };
            let r = run_spec(
                &s,
                ExperimentSpec::Search {
                    track,
                    method,
                    budget_ticks: ticks,
                    seed,
                    params,
                    warm_start: warm.map(Into::into),
                    physics: None,
                },
            )?;
            println!("{}", r.summary);
            println!(
                "trajectory: {}",
                r.trajectories
                    .first()
                    .map(|h| h.to_string())
                    .unwrap_or_default()
            );
        }
        Cmd::Verify { trajectory } => {
            let s = Session::open(&root)?;
            let r = run_spec(
                &s,
                ExperimentSpec::Verify {
                    trajectory: trajectory.into(),
                },
            )?;
            println!("{}", r.summary);
        }
        Cmd::Calibrate {
            track,
            ticks,
            probes,
        } => {
            let s = Session::open(&root)?;
            let r = run_spec(
                &s,
                ExperimentSpec::Calibrate {
                    tracks: track,
                    trajectories: vec![],
                    probe_runs: probes,
                    budget_ticks: ticks,
                    seed: 1,
                    physics: None,
                },
            )?;
            println!("{}", r.summary);
        }
        Cmd::Train {
            track,
            epochs,
            hidden,
        } => {
            let s = Session::open(&root)?;
            let r = run_spec(
                &s,
                ExperimentSpec::TrainBc {
                    tracks: track,
                    top_k: 10,
                    epochs,
                    hidden,
                    seed: 1,
                    value_head: true,
                },
            )?;
            println!("{}", r.summary);
        }
        Cmd::Experiment { spec } => {
            let s = Session::open(&root)?;
            let text = if let Some(p) = spec.strip_prefix('@') {
                std::fs::read_to_string(p)?
            } else {
                spec
            };
            let spec: ExperimentSpec = serde_json::from_str(&text)?;
            let r = run_spec(&s, spec)?;
            println!("{}", serde_json::to_string_pretty(&r.result)?);
            println!("{}", r.summary);
        }
        Cmd::Research {
            cycles,
            scripted,
            with_tasks,
        } => {
            let mut cfg = RtiConfig::load(&root)?;
            if scripted {
                cfg.research.brain = "scripted".into();
            }
            let s = Session::with_config(&root, cfg)?;
            let mut i = 0u32;
            loop {
                match rti_research::run_cycle(&s) {
                    Ok(r) => {
                        println!(
                            "cycle {} [{}] V={:.2} ${:.4}: {}",
                            r.cycle, r.brain, r.value_total, r.cost.llm_usd, r.conclusion.title
                        );
                        println!("  hypothesis: {}", r.proposal.hypothesis);
                        println!("  {}", r.report.summary);
                        for f in &r.follow_ups {
                            println!("  → {f}");
                        }
                    }
                    Err(e) => {
                        eprintln!("cycle failed: {e:#}");
                        if e.to_string().contains("budget") {
                            break;
                        }
                    }
                }
                if with_tasks {
                    while let Some((id, msg)) = rti_research::tasks::run_next_task(&s)? {
                        println!("  task {id}: {msg}");
                    }
                }
                i += 1;
                if cycles != 0 && i >= cycles {
                    break;
                }
            }
        }
        Cmd::Tasks { cmd } => {
            let s = Session::open(&root)?;
            match cmd {
                TaskCmd::List => {
                    for t in s.archive.tasks(50)? {
                        println!(
                            "[{}] {:<9} {:<8} prio {} cycle {:?}: {}",
                            t.id, t.status, t.kind, t.priority, t.cycle, t.title
                        );
                    }
                }
                TaskCmd::Run { n } => {
                    for _ in 0..n {
                        match rti_research::tasks::run_next_task(&s)? {
                            Some((id, msg)) => println!("task {id}: {msg}"),
                            None => {
                                println!("no pending tasks");
                                break;
                            }
                        }
                    }
                }
                TaskCmd::Add {
                    title,
                    description,
                    priority,
                } => {
                    let t = rti_archive::rows::TaskRow::new(
                        "coding",
                        &title,
                        &description,
                        priority,
                        None,
                    );
                    let id = s.archive.insert_task(&t)?;
                    println!("queued {id}");
                }
            }
        }
        Cmd::Code {
            title,
            description,
            no_bench_guard,
        } => {
            let s = Session::open(&root)?;
            let mut task = rti_agent::CodingTask::new(&title, &description);
            task.bench_guard = !no_bench_guard;
            let llm = if s.cfg.agent.backend == "builtin" {
                Some(&s.llm)
            } else {
                None
            };
            let out = rti_agent::run_task(&s.cfg.agent, llm, &s.root, &task, None, &s.repo_docs())?;
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Cmd::Query { sql } => {
            let s = Session::open(&root)?;
            for row in s.archive.query_json(&sql)? {
                println!("{}", serde_json::to_string(&row)?);
            }
        }
        Cmd::Ledger => {
            let s = Session::open(&root)?;
            println!(
                "{:<18} {:>10} {:>10} {:>9} {:>6} {:>9}",
                "category", "Mticks", "oracle", "wall s", "calls", "usd"
            );
            for (cat, c) in s.archive.ledger_by_category()? {
                println!(
                    "{:<18} {:>10.1} {:>10} {:>9.1} {:>6} {:>9.4}",
                    cat,
                    c.mticks(),
                    c.oracle_ticks,
                    c.wall_ms as f64 / 1000.0,
                    c.llm_calls,
                    c.llm_usd
                );
            }
            let t = s.archive.ledger_totals()?;
            println!(
                "{:<18} {:>10.1} {:>10} {:>9.1} {:>6} {:>9.4}",
                "TOTAL",
                t.mticks(),
                t.oracle_ticks,
                t.wall_ms as f64 / 1000.0,
                t.llm_calls,
                t.llm_usd
            );
            for m in s.archive.llm_call_stats()? {
                println!(
                    "  {:<40} {:>5} calls {:>9} in {:>9} out ${:.4}",
                    m.model, m.n, m.prompt_tokens, m.completion_tokens, m.usd
                );
            }
        }
        Cmd::Chat { role, prompt } => {
            let s = Session::open(&root)?;
            let resp = s.llm.chat(
                &role,
                &[rti_llm::Message::user(prompt)],
                &[],
                &Default::default(),
            )?;
            println!(
                "{}\n\n[{} | {} in / {} out | ${:.5} | {} ms]",
                resp.text(),
                resp.model,
                resp.usage.prompt_tokens,
                resp.usage.completion_tokens,
                resp.usage.usd,
                resp.latency_ms
            );
            s.flush_usage(None, None)?;
        }
        Cmd::Oracle => {
            let s = Session::open(&root)?;
            match s.oracle.available() {
                Ok(()) => println!(
                    "oracle {} available ({} ms/tick)",
                    s.oracle.name(),
                    s.oracle.ms_per_tick()
                ),
                Err(e) => println!("oracle {} unavailable: {e}", s.oracle.name()),
            }
        }
        Cmd::Serve { port } => {
            rti_server::serve(&root, port)?;
        }
        Cmd::Map { cmd } => match cmd {
            MapCmd::Search {
                name,
                author,
                tag,
                count,
            } => {
                let mut params: Vec<(String, String)> =
                    vec![("count".into(), count.min(50).to_string())];
                if let Some(v) = name {
                    params.push(("name".into(), v));
                }
                if let Some(v) = author {
                    params.push(("author".into(), v));
                }
                if let Some(v) = tag {
                    params.push(("tag".into(), v));
                }
                let refs: Vec<(&str, &str)> = params
                    .iter()
                    .map(|(a, b)| (a.as_str(), b.as_str()))
                    .collect();
                let r = rti_maps::TmxClient::new().search(&refs)?;
                for m in &r.results {
                    println!(
                        "{:>8}  {:<40} by {:<18} author {:>7} ms  WR {:>7}  awards {:>3}  {:?}",
                        m.map_id,
                        truncate(&m.name, 40),
                        truncate(&m.author_names().join(","), 18),
                        m.length,
                        m.wr_ms().map(|w| w.to_string()).unwrap_or("-".into()),
                        m.award_count,
                        m.tag_names()
                    );
                }
                if r.more {
                    println!("(more results available)");
                }
            }
            MapCmd::Import { source, name } => {
                let s = Session::open(&root)?;
                let r = run_spec(&s, ExperimentSpec::ImportMap { source, name })?;
                println!("{}", r.summary);
                if let Some(w) = r.result["warnings"].as_array() {
                    for w in w {
                        println!("  warning: {w}");
                    }
                }
            }
            MapCmd::Inspect { file } => {
                let m = rti_maps::parse_map(&std::fs::read(&file)?)?;
                println!("{}", serde_json::to_string_pretty(&m.info)?);
                println!(
                    "blocks: {}  items: {}  body chunks: {}",
                    m.blocks.len(),
                    m.items.len(),
                    m.body_chunks.len()
                );
                let cat = rti_maps::catalog::Catalog::load(&root)?;
                match rti_maps::compile_track(&m, &cat, "inspect") {
                    Ok((t, rep)) => println!(
                        "compile: {} m, {} nodes; {}",
                        t.length(),
                        t.nodes.len(),
                        serde_json::to_string_pretty(&rep)?
                    ),
                    Err(e) => println!("compile failed: {e}"),
                }
                for w in &m.warnings {
                    println!("warning: {w}");
                }
            }
            MapCmd::List => {
                let s = Session::open(&root)?;
                for m in s.archive.maps(100)? {
                    let cov = m
                        .report
                        .as_ref()
                        .map(|r| {
                            format!(
                                "{}/{} chained, finish={}",
                                r["blocks_chained"], r["blocks_recognized"], r["finish_found"]
                            )
                        })
                        .unwrap_or_default();
                    println!(
                        "{:<24} {:<30} by {:<16} tmx {:<7} author {:>7} ms  WR {:>7}  {}",
                        m.track_name,
                        truncate(&m.map_name, 30),
                        truncate(&m.author, 16),
                        m.tmx_id.map(|i| i.to_string()).unwrap_or("-".into()),
                        m.author_ms.unwrap_or(0),
                        m.wr_ms.map(|w| w.to_string()).unwrap_or("-".into()),
                        cov
                    );
                }
            }
        },
        Cmd::Export { trajectory } => {
            let s = Session::open(&root)?;
            let t = s
                .archive
                .trajectory(&trajectory.into())?
                .ok_or_else(|| anyhow::anyhow!("not found"))?;
            println!(
                "# {} on {} ({} ms, finished={})",
                t.method, t.track_name, t.result.time_ms, t.result.finished
            );
            let mut tick = 0u32;
            for r in rti_core::action::compress_actions(&t.actions) {
                println!(
                    "{}-{} steer {:+.3} gas {} brake {}",
                    tick * 10,
                    (tick + r.ticks) * 10,
                    r.action.steer,
                    r.action.gas as u8,
                    r.action.brake as u8
                );
                tick += r.ticks;
            }
        }
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

fn run_spec(s: &Session, spec: ExperimentSpec) -> anyhow::Result<rti_research::ExperimentReport> {
    let mut spec = spec;
    rti_research::cycle::validate_spec(s, &mut spec)?;
    let id = s
        .archive
        .begin_experiment(&spec, None, None, &Provenance::now())?;
    match rti_research::run_experiment(s, &spec, &id) {
        Ok(r) => {
            s.archive
                .finish_experiment(&id, "done", &r.result, &r.cost, None, None, &r.summary)?;
            s.archive.ledger_add(spec.kind(), &id, &r.cost, "cli")?;
            Ok(r)
        }
        Err(e) => {
            s.archive.finish_experiment(
                &id,
                "failed",
                &serde_json::json!({"error": e.to_string()}),
                &Default::default(),
                None,
                None,
                &format!("failed: {e}"),
            )?;
            Err(e)
        }
    }
}

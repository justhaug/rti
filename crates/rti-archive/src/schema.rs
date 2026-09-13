//! DuckDB schema. All timestamps are TEXT (RFC3339) and all structured
//! payloads are TEXT holding JSON, which keeps binding trivial and lets
//! the LLM query with DuckDB's JSON functions.

pub const SCHEMA_VERSION: i64 = 1;

pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (version BIGINT NOT NULL, applied_at TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS tracks (
    hash TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    length_m DOUBLE NOT NULL,
    n_nodes BIGINT NOT NULL,
    json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS physics (
    hash TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    version BIGINT NOT NULL,
    json TEXT NOT NULL,
    source_experiment TEXT,
    oracle_mean_err DOUBLE,
    is_current BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS experiments (
    id TEXT PRIMARY KEY,
    cycle BIGINT,
    kind TEXT NOT NULL,
    method TEXT,
    track TEXT,
    spec_json TEXT NOT NULL,
    hypothesis_id TEXT,
    status TEXT NOT NULL,
    result_json TEXT,
    summary TEXT,
    cost_json TEXT,
    value_json TEXT,
    value_total DOUBLE,
    provenance_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS trajectories (
    hash TEXT PRIMARY KEY,
    track_name TEXT NOT NULL,
    track_hash TEXT NOT NULL,
    world TEXT NOT NULL,
    method TEXT,
    experiment_id TEXT,
    finished BOOLEAN NOT NULL,
    time_ms BIGINT NOT NULL,
    progress DOUBLE NOT NULL,
    ticks BIGINT NOT NULL,
    has_states BOOLEAN NOT NULL,
    parent TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS verifications (
    id TEXT PRIMARY KEY,
    trajectory_hash TEXT NOT NULL,
    oracle TEXT NOT NULL,
    oracle_trajectory_hash TEXT,
    experiment_id TEXT,
    divergence_json TEXT NOT NULL,
    mean_pos_err DOUBLE NOT NULL,
    max_pos_err DOUBLE NOT NULL,
    time_ms_sim BIGINT NOT NULL,
    time_ms_oracle BIGINT NOT NULL,
    both_finished BOOLEAN NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS models (
    hash TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    experiment_id TEXT,
    meta_json TEXT NOT NULL,
    metrics_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS findings (
    id TEXT PRIMARY KEY,
    cycle BIGINT,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    confidence DOUBLE NOT NULL,
    experiment_ids_json TEXT NOT NULL,
    tags_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hypotheses (
    id TEXT PRIMARY KEY,
    cycle BIGINT,
    text TEXT NOT NULL,
    rationale TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ledger (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    ref_id TEXT NOT NULL,
    sim_ticks BIGINT NOT NULL,
    oracle_ticks BIGINT NOT NULL,
    wall_ms BIGINT NOT NULL,
    cpu_ms BIGINT NOT NULL,
    llm_calls BIGINT NOT NULL,
    llm_prompt_tokens BIGINT NOT NULL,
    llm_completion_tokens BIGINT NOT NULL,
    llm_usd DOUBLE NOT NULL,
    note TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS llm_calls (
    id TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_tokens BIGINT NOT NULL,
    completion_tokens BIGINT NOT NULL,
    usd DOUBLE NOT NULL,
    latency_ms BIGINT NOT NULL,
    cycle BIGINT,
    experiment_id TEXT,
    ok BOOLEAN NOT NULL,
    error TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    cycle BIGINT,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER NOT NULL,
    branch TEXT,
    result_json TEXT,
    cost_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cycles (
    id BIGINT PRIMARY KEY,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    summary TEXT,
    cost_json TEXT,
    brain TEXT NOT NULL,
    model TEXT NOT NULL
);
"#;

/// Human-readable schema summary handed to LLM roles alongside the
/// `query_archive` tool. Keep in sync with `DDL`.
pub const SCHEMA_DOC: &str = r#"RTI archive (DuckDB). All *_json columns are TEXT containing JSON (use json_extract / ->> ). Timestamps are RFC3339 TEXT.

tracks(hash, name, length_m, n_nodes, json, created_at)
physics(hash, label, version, json, source_experiment, oracle_mean_err, is_current, created_at)
    -- json is PhysicsParams; exactly one row has is_current = true
experiments(id, cycle, kind, method, track, spec_json, hypothesis_id, status, result_json, summary, cost_json, value_json, value_total, provenance_json, started_at, finished_at)
    -- kind: search|verify|calibrate|train_bc|benchmark|generate_track; status: running|done|failed
    -- cost_json is CostRecord {sim_ticks, oracle_ticks, wall_ms, cpu_ms, llm_calls, llm_prompt_tokens, llm_completion_tokens, llm_usd}
    -- value_json is ValueScore {race_improvement, information_gain, novelty, sim_accuracy, efficiency}
trajectories(hash, track_name, track_hash, world, method, experiment_id, finished, time_ms, progress, ticks, has_states, parent, created_at)
    -- world: 'sim:<params hash prefix>' or 'oracle:<name>'; full inputs/telemetry live in the CAS under hash
verifications(id, trajectory_hash, oracle, oracle_trajectory_hash, experiment_id, divergence_json, mean_pos_err, max_pos_err, time_ms_sim, time_ms_oracle, both_finished, created_at)
models(hash, kind, experiment_id, meta_json, metrics_json, created_at)
    -- kind: policy|value|policy_bundle; weights in CAS under hash
findings(id, cycle, kind, title, body, confidence, experiment_ids_json, tags_json, created_at)
    -- kind: result|insight|anomaly|sim_gap|method
hypotheses(id, cycle, text, rationale, status, created_at, updated_at)
    -- status: open|supported|refuted|abandoned
ledger(id, category, ref_id, sim_ticks, oracle_ticks, wall_ms, cpu_ms, llm_calls, llm_prompt_tokens, llm_completion_tokens, llm_usd, note, created_at)
    -- category: experiment|llm|task|cycle
llm_calls(id, role, model, prompt_tokens, completion_tokens, usd, latency_ms, cycle, experiment_id, ok, error, created_at)
tasks(id, cycle, kind, title, description, status, priority, branch, result_json, cost_json, created_at, updated_at)
    -- kind: coding|verify|calibrate; status: pending|running|done|failed|merged|rejected
cycles(id, started_at, finished_at, summary, cost_json, brain, model)
"#;

use chrono::{Duration as CDur, TimeZone, Utc};
use rti_core::config::CloudConfig;
use rti_infra::budget::{decide, Budget, BudgetState, SpendDecision, SpendKind};
use rti_infra::fly::{FlyClient, MachineSpec};
use rti_infra::oracle_lifecycle::idle_expired;
use rti_infra::runpod::RunpodClient;
use rti_infra::watchdog::should_stop;
use std::io::Read;

#[test]
fn budget_caps_and_approval() {
    let cfg = CloudConfig {
        compute_daily_usd: 5.0,
        compute_monthly_usd: 20.0,
        require_approval_above_usd: 3.0,
        oracle_daily_hours: 2.0,
        ..Default::default()
    };
    let st = BudgetState {
        compute_usd_today: 4.0,
        compute_usd_month: 4.0,
        ..Default::default()
    };
    assert_eq!(
        decide(&cfg, &st, SpendKind::Compute, 0.5),
        SpendDecision::Ok
    );
    assert!(matches!(
        decide(&cfg, &st, SpendKind::Compute, 1.5),
        SpendDecision::Denied(_)
    ));
    let st2 = BudgetState {
        compute_usd_month: 18.0,
        ..Default::default()
    };
    assert!(
        matches!(decide(&cfg, &st2, SpendKind::Compute, 4.0), SpendDecision::Denied(m) if m.contains("monthly"))
    );
    let st3 = BudgetState::default();
    assert!(matches!(
        decide(&cfg, &st3, SpendKind::Compute, 4.0),
        SpendDecision::NeedsApproval(_)
    ));
    let st4 = BudgetState {
        oracle_seconds_today: 3600.0 * 1.9,
        ..Default::default()
    };
    assert!(matches!(
        decide(&cfg, &st4, SpendKind::Oracle, 0.5),
        SpendDecision::Denied(_)
    ));
    assert_eq!(
        decide(&cfg, &st4, SpendKind::Oracle, 0.05),
        SpendDecision::Ok
    );
}

#[test]
fn budget_persists_and_rolls_over() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = CloudConfig::default();
    let b = Budget::open(&cfg, dir.path());
    b.record_compute_usd(1.25);
    b.record_oracle_seconds(600.0);
    let b2 = Budget::open(&cfg, dir.path());
    let st = b2.state();
    assert!((st.compute_usd_today - 1.25).abs() < 1e-9);
    assert!((st.oracle_seconds_today - 600.0).abs() < 1e-9);
    // rollover to next day keeps monthly, clears daily
    let tomorrow = Utc::now() + CDur::days(1);
    b2.rollover(tomorrow);
    let st = b2.state();
    assert_eq!(st.compute_usd_today, 0.0);
    if st.month == format!("{}", (Utc::now()).format("%Y-%m")) {
        assert!((st.compute_usd_month - 1.25).abs() < 1e-9);
    }
    // far future month clears monthly too
    b2.rollover(Utc::now() + CDur::days(60));
    assert_eq!(b2.state().compute_usd_month, 0.0);
}

#[test]
fn idle_and_watchdog_decisions() {
    let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
    assert!(!idle_expired(Some(t0), t0 + CDur::minutes(9), 10));
    assert!(idle_expired(Some(t0), t0 + CDur::minutes(10), 10));
    assert!(idle_expired(None, t0, 10));
    // watchdog: boot grace protects a pod that never got a heartbeat yet
    assert!(!should_stop(t0, None, t0 + CDur::minutes(12), 10, 15));
    assert!(should_stop(t0, None, t0 + CDur::minutes(30), 10, 15));
    assert!(!should_stop(
        t0,
        Some(t0 + CDur::minutes(25)),
        t0 + CDur::minutes(30),
        10,
        15
    ));
    assert!(should_stop(
        t0,
        Some(t0 + CDur::minutes(19)),
        t0 + CDur::minutes(30),
        10,
        15
    ));
}

/// Mock server answering Runpod GraphQL and Fly REST shapes.
fn mock_server() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let l = log.clone();
    std::thread::spawn(move || {
        let mut running = false;
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let url = req.url().to_string();
            l.lock().unwrap().push(format!("{} {}", req.method(), url));
            let resp = if url.starts_with("/graphql") {
                let auth = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.to_string());
                if auth.as_deref() != Some("Bearer testkey") {
                    r#"{"errors":[{"message":"unauthorized"}]}"#.to_string()
                } else if body.contains("podResume") {
                    running = true;
                    r#"{"data":{"podResume":{"id":"pod1","desiredStatus":"RUNNING"}}}"#.to_string()
                } else if body.contains("podStop") {
                    running = false;
                    r#"{"data":{"podStop":{"id":"pod1","desiredStatus":"EXITED"}}}"#.to_string()
                } else if running {
                    r#"{"data":{"pod":{"id":"pod1","name":"oracle","desiredStatus":"RUNNING","costPerHr":0.44,"runtime":{"uptimeInSeconds":42,"ports":[{"ip":"1.2.3.4","isIpPublic":true,"privatePort":27015,"publicPort":31234,"type":"tcp"}]}}}}"#.to_string()
                } else {
                    r#"{"data":{"pod":{"id":"pod1","name":"oracle","desiredStatus":"EXITED","costPerHr":0.44,"runtime":null}}}"#.to_string()
                }
            } else if url == "/v1/apps/rti/machines" && req.method() == &tiny_http::Method::Post {
                r#"{"id":"m1","name":"rti-worker-x","state":"created","region":"iad"}"#.to_string()
            } else if url == "/v1/apps/rti/machines" {
                r#"[{"id":"m1","name":"rti-worker-x","state":"started","region":"iad"}]"#
                    .to_string()
            } else if url.starts_with("/v1/apps/rti/machines/m1/stop") {
                "{}".to_string()
            } else if url.starts_with("/v1/apps/rti/machines/m1")
                && req.method() == &tiny_http::Method::Delete
            {
                "{}".to_string()
            } else if url.starts_with("/v1/apps/rti/machines/m1") {
                r#"{"id":"m1","name":"rti-worker-x","state":"stopped","region":"iad","events":[{"request":{"exit_event":{"exit_code":0}}}]}"#.to_string()
            } else {
                r#"{"error":"nope"}"#.to_string()
            };
            let _ = req.respond(tiny_http::Response::from_string(resp).with_header(
                tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
            ));
        }
    });
    (port, log)
}

#[test]
fn runpod_client_lifecycle_against_mock() {
    let (port, _log) = mock_server();
    let url = format!("http://127.0.0.1:{port}/graphql");
    let bad = RunpodClient::new(&url, "wrong");
    let err = bad.pod_status("pod1").unwrap_err().to_string();
    assert!(err.contains("unauthorized"), "{err}");
    let c = RunpodClient::new(&url, "testkey");
    let st = c.pod_status("pod1").unwrap();
    assert!(!st.running);
    assert_eq!(c.resume_pod("pod1", 1).unwrap(), "RUNNING");
    let st = c.pod_status("pod1").unwrap();
    assert!(st.running);
    assert_eq!(st.public_endpoint(27015), Some(("1.2.3.4".into(), 31234)));
    assert_eq!(st.cost_per_hr, 0.44);
    assert_eq!(c.stop_pod("pod1").unwrap(), "EXITED");
    assert!(!c.pod_status("pod1").unwrap().running);
}

#[test]
fn fly_client_against_mock() {
    let (port, log) = mock_server();
    let f = FlyClient::new(&format!("http://127.0.0.1:{port}/v1"), "tok");
    let ms = f.list_machines("rti").unwrap();
    assert_eq!(ms.len(), 1);
    let spec = MachineSpec {
        image: "img".into(),
        cmd: vec!["rti".into(), "bench".into()],
        size: "performance-4x".into(),
        region: "iad".into(),
        env: Default::default(),
        auto_destroy: true,
        name: Some("rti-worker-t".into()),
    };
    let m = f.create_machine("rti", &spec).unwrap();
    assert_eq!(m.id, "m1");
    let m = f
        .wait_for_state("rti", "m1", "stopped", std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(m.exit_code, Some(0));
    f.stop_machine("rti", "m1").unwrap();
    f.destroy_machine("rti", "m1").unwrap();
    let l = log.lock().unwrap();
    assert!(l
        .iter()
        .any(|x| x.starts_with("POST /v1/apps/rti/machines")));
    assert!(l
        .iter()
        .any(|x| x.starts_with("DELETE /v1/apps/rti/machines/m1")));
}

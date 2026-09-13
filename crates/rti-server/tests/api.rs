use std::time::{Duration, Instant};

fn get(port: u16, path: &str) -> serde_json::Value {
    ureq::get(&format!("http://127.0.0.1:{port}{path}"))
        .call()
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap()
}

fn post(port: u16, path: &str, body: serde_json::Value) -> serde_json::Value {
    ureq::post(&format!("http://127.0.0.1:{port}{path}"))
        .send_json(&body)
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap()
}

#[test]
fn api_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("rti.toml"),
        "[research]\nbrain = \"scripted\"\n[budget]\ndefault_ticks = 1000000\n",
    )
    .unwrap();
    let (port, _state) = rti_server::serve_on(dir.path(), "127.0.0.1:0").unwrap();

    let html = ureq::get(&format!("http://127.0.0.1:{port}/"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    assert!(html.contains("<html") && html.contains("RTI"));

    let s = get(port, "/api/status");
    assert_eq!(s["tracks"].as_i64().unwrap(), 5);
    assert_eq!(s["loop"]["running"], false);
    assert!(s["oracle"].as_str().unwrap().starts_with("hidden_sim"));

    let tracks = get(port, "/api/tracks");
    assert!(tracks
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "hairpin"));
    let t = get(port, "/api/track/hairpin");
    assert!(t["track"]["nodes"].as_array().unwrap().len() > 10);

    let r = post(
        port,
        "/api/experiment",
        serde_json::json!({"spec": {"kind": "benchmark", "ticks": 1000000}}),
    );
    assert!(r["result"]["mticks_per_s"].as_f64().unwrap() > 0.0, "{r}");

    let before = get(port, "/api/status")["cycles"].as_i64().unwrap();
    let once = post(port, "/api/loop/once", serde_json::json!({}));
    assert!(once["report"]["cycle"].is_number(), "{once}");
    let start = Instant::now();
    loop {
        let now = get(port, "/api/status")["cycles"].as_i64().unwrap();
        if now > before {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "cycle count did not increment"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    let exps = get(port, "/api/experiments?n=10");
    assert!(exps.as_array().unwrap().len() >= 2);
    let id = exps[0]["id"].as_str().unwrap().to_string();
    let one = get(port, &format!("/api/experiment/{id}"));
    assert_eq!(one["experiment"]["id"], id);

    let q = post(
        port,
        "/api/query",
        serde_json::json!({"sql": "select count(*) as n from experiments"}),
    );
    assert!(q["rows"][0]["n"].as_i64().unwrap() >= 2);

    let bad = ureq::post(&format!("http://127.0.0.1:{port}/api/query"))
        .send_json(&serde_json::json!({"sql": "drop table experiments"}));
    assert!(bad.is_err() || bad.unwrap().status() == 400);

    let ev = get(port, "/api/events?since=0");
    assert!(!ev["events"].as_array().unwrap().is_empty());

    let task = post(
        port,
        "/api/tasks",
        serde_json::json!({"kind": "coding", "title": "t", "description": "d", "priority": 1}),
    );
    assert!(task["id"].is_string());
    let tasks = get(port, "/api/tasks");
    assert!(tasks.as_array().unwrap().iter().any(|t| t["title"] == "t"));

    let chat = ureq::post(&format!("http://127.0.0.1:{port}/api/chat"))
        .send_json(&serde_json::json!({"message": "hi"}));
    // no API key in tests: expect a 400 with a helpful message
    if std::env::var("OPENROUTER_API_KEY").is_err() {
        assert!(chat.is_err());
    }
}

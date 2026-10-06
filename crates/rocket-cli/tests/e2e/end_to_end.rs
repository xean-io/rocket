use std::net::TcpListener;
use std::time::Duration;

use crate::common::*;

#[test]
fn end_to_end() {
    let Some(e) = setup() else { return };

    step("up all starts everything healthy");
    let res = e.json(&[], 0, &["up", "all"]);
    let services = arr(&res, "services");
    assert_eq!(services.len(), 4, "unexpected result {res}");
    for s in &services {
        assert!(
            s["action"] == "started" && s["state"] == "running",
            "{}: {}/{} {}",
            s["service"],
            s["action"],
            s["state"],
            s["error"]
        );
    }
    assert!(
        http_ok(18431) && http_ok(18432) && http_ok(18433),
        "fixture servers not reachable"
    );
    let again = e.json(&[], 0, &["up", "all"]);
    for s in arr(&again, "services") {
        assert_eq!(
            s["action"], "already_running",
            "second up: {} {}",
            s["service"], s["action"]
        );
    }

    step("dotenv reaches the child and logs are captured");
    wait_for(Duration::from_secs(5), "sleeper log", || {
        let logs = e.json(&[], 0, &["logs", "sleeper", "--tail", "20"]);
        tail_has(&strings(&logs, "lines"), "greeting=hello")
    });

    step("ports lists global leases");
    let res = e.json(&[], 0, &["ports"]);
    let got: std::collections::HashMap<i64, String> = arr(&res, "ports")
        .iter()
        .map(|p| {
            (
                p["port"].as_i64().unwrap(),
                str_of(p, "service").to_string(),
            )
        })
        .collect();
    assert!(
        got.get(&18431).map(String::as_str) == Some("static")
            && got.get(&18432).map(String::as_str) == Some("web")
            && got.get(&18433).map(String::as_str) == Some("fixed"),
        "ports {res}"
    );

    step("down all frees every port");
    let res = e.json(&[], 0, &["down", "--all"]);
    let stopped = arr(&res, "stopped");
    assert!(
        stopped.len() == 4 && str_of(stopped.last().unwrap(), "service") != "sleeper",
        "stopped {res}"
    );
    for p in [18431, 18432, 18433] {
        assert!(port_free(p), "port {p} still busy");
    }
    let ports = e.json(&[], 0, &["ports"]);
    assert!(arr(&ports, "ports").is_empty(), "leases left {ports}");

    step("busy port is remapped via env, or fails without env");
    let l1 = TcpListener::bind("127.0.0.1:18431").unwrap();
    let res = e.json(&[], 0, &["up", "static"]);
    let s = &res["services"][0];
    let remaps = arr(s, "remaps");
    assert!(
        s["ports"]["http"] == 18531
            && remaps.len() == 1
            && remaps[0]["holder"]["pid"] == std::process::id(),
        "remap {s}"
    );
    assert!(http_ok(18531), "remapped server not reachable");

    let l2 = TcpListener::bind("127.0.0.1:18433").unwrap();
    let failed = e.json(&[], 2, &["up", "fixed"]);
    let f = &failed["services"][0];
    assert!(
        f["action"] == "failed" && str_of(f, "error").contains("no env var"),
        "fixed {f}"
    );
    e.json(&[], 0, &["down", "--all"]);
    drop((l1, l2));

    step("owners and ttl");
    e.json(&[], 0, &["up", "static"]);
    e.json(&[("ROCKET_OWNER", "agent:t2")], 0, &["up", "web"]);
    e.json(
        &[("ROCKET_OWNER", "agent:t1")],
        0,
        &["up", "sleeper", "--ttl", "2s"],
    );
    let ps = e.ps();
    assert!(
        ps["static"]["owner"] == "user"
            && ps["web"]["owner"] == "agent:t2"
            && ps["sleeper"]["owner"] == "agent:t1",
        "owners {ps:?}"
    );
    let down = e.json(&[], 0, &["down", "--owner", "agent:t2"]);
    let stopped = arr(&down, "stopped");
    assert!(
        stopped.len() == 1 && stopped[0]["service"] == "web",
        "owner down {down}"
    );
    wait_for(Duration::from_secs(15), "ttl expiry", || {
        e.ps()["sleeper"]["state"] == "stopped"
    });
    assert_eq!(
        e.ps()["static"]["state"],
        "running",
        "user's static must survive agent cleanup"
    );

    step("daemon crash: reconcile adopts live and marks dead");
    e.json(&[], 0, &["up", "web"]);
    let ps = e.ps();
    let web_pid = ps["web"]["pid"].as_i64().unwrap();
    let st = e.json(&[], 0, &["daemon", "status"]);
    sigkill(st["info"]["pid"].as_i64().unwrap() as i32);
    sigkill_group(web_pid); // service dies while the daemon is down
    wait_for(Duration::from_secs(5), "daemon gone", || {
        e.rocket(&[], &["daemon", "status"]).1 == 1
    });
    e.json(&[], 0, &["daemon", "start"]);
    let ps = e.ps();
    assert!(
        ps["static"]["state"] == "running" && http_ok(18431),
        "static should be adopted: {}",
        ps["static"]
    );
    assert_eq!(
        ps["web"]["state"], "dead",
        "web should be dead: {}",
        ps["web"]
    );
    e.json(&[], 0, &["gc"]);
    let down = e.json(&[], 0, &["down", "--everywhere"]);
    let stopped = arr(&down, "stopped");
    assert!(
        stopped.len() == 1 && stopped[0]["service"] == "static",
        "down everywhere {down}"
    );
    for p in FIXTURE_PORTS {
        assert!(port_free(p), "port {p} still busy");
    }
}

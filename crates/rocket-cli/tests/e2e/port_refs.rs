use std::net::TcpListener;
use std::time::Duration;

use crate::common::*;

#[test]
fn port_references_propagate_real_remap() {
    let Some(e) = setup() else { return };
    e.write_manifest(
        r#"version: 1
name: rocket-fixture
services:
  static:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    ports: {http: {default: 18431, env: {PORT: "{port}"}}}
    health: {http: /, timeout: 20s}
  consumer:
    run: 'echo provider_url=$API_URL; echo shell_literal=$SHELL_LITERAL; exec sleep 600'
    env:
      API_URL: "http://127.0.0.1:{static.http}"
      SHELL_LITERAL: "${static.http}"
"#,
    );
    let _holder = TcpListener::bind("127.0.0.1:18431").unwrap();
    let up = e.json(&[], 0, &["up", "consumer"]);
    let svcs = arr(&up, "services");
    assert!(
        svcs.len() == 2 && svcs[0]["service"] == "static" && svcs[1]["service"] == "consumer",
        "implicit dependency start order: {up}"
    );
    assert!(
        svcs[0]["ports"]["http"] == 18531 && http_ok(18531),
        "provider did not start at remapped port: {}",
        svcs[0]
    );
    wait_for(
        Duration::from_secs(5),
        "consumer resolved environment",
        || {
            let logs = e.json(&[], 0, &["logs", "consumer"]);
            let lines = strings(&logs, "lines").join("\n");
            lines.contains("provider_url=http://127.0.0.1:18531")
                && lines.contains("shell_literal=${static.http}")
        },
    );
    let down = e.json(&[], 0, &["down", "--all"]);
    let stopped = arr(&down, "stopped");
    assert!(
        stopped.len() == 2 && stopped[0]["service"] == "consumer",
        "reverse dependency shutdown: {down}"
    );
    for (name, run) in e.ps() {
        assert_eq!(
            run["state"], "stopped",
            "test-owned {name} still active: {run}"
        );
    }
    let ports = e.json(&[], 0, &["ports"]);
    assert!(
        arr(&ports, "ports").is_empty(),
        "test leases remain: {ports}"
    );
}

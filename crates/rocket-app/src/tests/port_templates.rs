//! Ports of `port_templates_test.go`.

use crate::testing::*;
use rocket_domain::api::DownRequest;
use rocket_domain::ports::Store;

const ROOT: &str = "/code/templates";

const TEMPLATE_MANIFEST: &str = r#"version: 1
name: templates
services:
  api:
    task: api:dev
    env: SERVICE_ENV
    ports:
      http:
        default: 8080
        env:
          PORT: "{port}"
          API_BIND: "0.0.0.0:{port}"
          PUBLIC_URL: {default: "http://localhost:{port}"}
"#;

struct Case {
    name: &'static str,
    base: &'static [&'static str],
    dotenv: &'static [(&'static str, &'static str)],
    service_env: &'static str,
    want_url: &'static str,
}

#[tokio::test]
async fn up_port_env_templates() {
    let cases = [
        Case {
            name: "unset default",
            base: &[],
            dotenv: &[],
            service_env: "{}",
            want_url: "http://localhost:8180",
        },
        Case {
            name: "base override",
            base: &["PUBLIC_URL=https://base.example"],
            dotenv: &[],
            service_env: "{}",
            want_url: "https://base.example",
        },
        Case {
            name: "dotenv override",
            base: &["PUBLIC_URL=https://base.example"],
            dotenv: &[("PUBLIC_URL", "https://dotenv.example")],
            service_env: "{}",
            want_url: "https://dotenv.example",
        },
        Case {
            name: "service override",
            base: &[],
            dotenv: &[("PUBLIC_URL", "https://dotenv.example")],
            service_env: r#"{PUBLIC_URL: "https://service.example"}"#,
            want_url: "https://service.example",
        },
        Case {
            name: "empty override falls back",
            base: &["PUBLIC_URL=https://base.example"],
            dotenv: &[("PUBLIC_URL", "")],
            service_env: "{}",
            want_url: "http://localhost:8180",
        },
    ];
    for tc in cases {
        let yaml = TEMPLATE_MANIFEST.replace("SERVICE_ENV", tc.service_env);
        let p = rocket_manifest::parse(yaml.as_bytes(), ROOT)
            .unwrap_or_else(|e| panic!("{}: parse port templates: {e}", tc.name));
        let h = Harness::new();
        h.loader.set(p);
        h.env.set(tc.dotenv);
        let mut base = vec!["PORT=9999", "API_BIND=ignored", "ROCKET_OWNER=agent:test"];
        base.extend(tc.base);
        h.set_base_env(&base);
        h.probe.set_busy(8080, holder(77, "foreign"));
        let res = h.up(req(ROOT, &[])).await;
        assert!(
            !res.failed() && res.services.len() == 1 && res.services[0].ports["http"] == 8180,
            "{}: up result: {res:?}",
            tc.name
        );
        let remaps = &res.services[0].remaps;
        assert!(
            remaps.len() == 1
                && remaps[0].env == "API_BIND"
                && remaps[0].holder.as_ref().unwrap().pid == 77,
            "{}: remap env must remain a representative string: {remaps:?}",
            tc.name
        );
        let spec = h.runner.spec_for(ROOT);
        assert_eq!(spec.argv, ["task", "api:dev"], "{}: task argv", tc.name);
        let env = env_map(&spec.env);
        for (name, want) in [
            ("PORT", "8180"),
            ("API_BIND", "0.0.0.0:8180"),
            ("PUBLIC_URL", tc.want_url),
        ] {
            assert_eq!(env[name], want, "{}: {name}", tc.name);
        }
        assert!(
            !env.contains_key("ROCKET_OWNER"),
            "{}: inherited ROCKET_OWNER leaked",
            tc.name
        );
        h.app
            .down(DownRequest {
                project: ROOT.into(),
                ..DownRequest::default()
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn up_default_port_binding_cannot_remap() {
    let p = rocket_manifest::parse(
        br#"version: 1
name: templates
services: {api: {run: dev, ports: {http: {default: 8080, env: {PUBLIC_URL: {default: "http://localhost:{port}"}}}}}}
"#,
        ROOT,
    )
    .expect("parse default port binding");
    let h = Harness::new();
    h.loader.set(p);
    h.probe.set_busy(8080, holder(77, ""));
    let res = h.up(req(ROOT, &[])).await;
    assert!(
        res.failed() && h.runner.started().is_empty(),
        "default-only binding must not authorize remap: {res:?}"
    );
    let leases = h.store.list_leases().unwrap();
    assert!(leases.is_empty(), "failed start leaked leases: {leases:?}");
}

//! up, restart, down, ps, logs, ports and gc (Go: `services.go`).

use crate::cli::{DownArgs, LogsArgs, PsArgs, RestartArgs, UpArgs};
use crate::error::{CliError, EXIT_PARTIAL, Result};
use crate::format::{
    errln, expires_str_now, format_ports, json_line, or_dash, out, outln, pid_str, print_json,
};
use crate::globals::{Globals, daemon_client, owner};
use crate::stream::{Interrupt, follow};
use crate::tabwriter::Table;
use rocket_domain::api::{DownRequest, UpRequest, UpResult};

/// What `up` and `restart` share.
pub struct UpParams {
    services: Vec<String>,
    env: String,
    profiles: Vec<String>,
    owner: String,
    ttl: String,
    restart: bool,
}

impl From<UpArgs> for UpParams {
    fn from(a: UpArgs) -> Self {
        Self {
            services: a.services,
            env: a.env,
            profiles: a.profiles,
            owner: a.owner,
            ttl: a.ttl,
            restart: false,
        }
    }
}

impl From<RestartArgs> for UpParams {
    fn from(a: RestartArgs) -> Self {
        Self {
            services: a.services,
            env: a.env,
            profiles: a.profiles,
            owner: a.owner,
            ttl: a.ttl,
            restart: true,
        }
    }
}

/// The `up` result as `rocket up` prints it (Go: `writeUp`).
pub fn render_up(res: &UpResult) -> String {
    let mut s = format!("project {} (env {})\n", res.project, res.env);
    let mut t = Table::new();
    t.row(["SERVICE", "ACTION", "STATE", "PORTS", "NOTE"]);
    for svc in &res.services {
        let mut notes: Vec<String> = svc
            .remaps
            .iter()
            .map(|r| {
                format!(
                    "{} {}->{} via {} ({})",
                    r.name, r.from, r.to, r.env, r.reason
                )
            })
            .collect();
        if !svc.error.is_empty() {
            notes.push(svc.error.clone());
        }
        t.row([
            svc.service.clone(),
            svc.action.clone(),
            svc.state.to_string(),
            format_ports(&svc.ports),
            notes.join("; "),
        ]);
    }
    s.push_str(&t.render());
    for hint in &res.hints {
        s.push_str(&format!("hint: {hint}\n"));
    }
    s
}

pub async fn up(g: &Globals, p: UpParams) -> Result<()> {
    let project = g.project_ref()?;
    let client = daemon_client().await?;
    let req = UpRequest {
        project,
        services: p.services,
        env: p.env,
        profiles: p.profiles,
        owner: owner(&p.owner),
        ttl: p.ttl,
    };
    let res = if p.restart {
        client.restart(&req).await?
    } else {
        client.up(&req).await?
    };
    if g.json {
        print_json(&res);
    } else {
        out(&render_up(&res));
    }
    if res.failed() {
        return Err(CliError::Exit(EXIT_PARTIAL));
    }
    Ok(())
}

/// Validates `down`'s selection and resolves the project (everything that
/// happens before the daemon is contacted).
pub fn down_request(g: &Globals, cwd: &std::path::Path, a: DownArgs) -> Result<DownRequest> {
    if a.services.is_empty() && !a.all && !a.everywhere && a.owner.is_empty() {
        return Err(CliError::msg(
            "nothing selected: pass services/groups, --all, --owner <owner> or --everywhere",
        ));
    }
    let mut req = DownRequest {
        services: a.services,
        owner: a.owner,
        everywhere: a.everywhere,
        ..DownRequest::default()
    };
    if !a.everywhere {
        match g.project_ref_in(cwd) {
            Ok(project) => req.project = project,
            Err(e) => {
                if a.all && g.project.is_empty() && e.is_no_manifest() {
                    return Err(CliError::NoManifest(format!(
                        "{e}; use --everywhere to stop services across all registered projects"
                    )));
                }
                return Err(e);
            }
        }
    }
    Ok(req)
}

pub async fn down(g: &Globals, a: DownArgs) -> Result<()> {
    let req = down_request(g, &std::env::current_dir()?, a)?;
    let client = daemon_client().await?;
    let res = client.down(&req).await?;
    if g.json {
        print_json(&res);
    } else {
        if res.stopped.is_empty() {
            outln("nothing running");
        }
        for r in &res.stopped {
            outln(&format!("stopped {}/{}", r.project, r.service));
        }
        for n in &res.compose_down {
            outln(&format!("compose down {n}"));
        }
        for e in &res.errors {
            errln(&format!("error: {e}"));
        }
    }
    if !res.errors.is_empty() {
        return Err(CliError::Exit(EXIT_PARTIAL));
    }
    Ok(())
}

/// The project of `ps`/`jobs`/`status`: an unresolvable one is only an error
/// when `-p` named it, otherwise the command spans all projects.
pub fn optional_project(g: &Globals) -> Result<Option<String>> {
    match g.project_ref() {
        Ok(r) => Ok(Some(r)),
        Err(e) if !g.project.is_empty() => Err(e),
        Err(_) => Ok(None),
    }
}

pub async fn ps(g: &Globals, a: PsArgs) -> Result<()> {
    let project = if a.all_projects {
        None
    } else {
        optional_project(g)?
    };
    let client = daemon_client().await?;
    let res = client.ps(project.as_deref(), a.all_projects).await?;
    if g.json {
        print_json(&res);
        return Ok(());
    }
    let mut t = Table::new();
    t.row([
        "PROJECT", "SERVICE", "KIND", "STATE", "HEALTH", "PID", "PORTS", "OWNER", "TTL",
    ]);
    for r in &res.services {
        t.row([
            r.project.clone(),
            r.service.clone(),
            r.kind.to_string(),
            r.state.to_string(),
            or_dash(r.health.as_str()),
            pid_str(r.pid),
            format_ports(&r.ports),
            or_dash(&r.owner),
            expires_str_now(r.expires_at),
        ]);
    }
    out(&t.render());
    Ok(())
}

pub async fn logs(g: &Globals, a: LogsArgs) -> Result<()> {
    let project = g.project_ref()?;
    let client = daemon_client().await?;
    if !a.follow {
        let res = client.logs(&project, &a.service, a.tail).await?;
        if g.json {
            print_json(&res);
        } else {
            for l in &res.lines {
                outln(l);
            }
        }
        return Ok(());
    }
    let stream = client
        .follow_logs(&project, &a.service, Some(a.tail))
        .await?;
    let mut interrupt = Interrupt::new()?;
    let json = g.json;
    follow(stream, &mut interrupt, |e| {
        if json {
            outln(&json_line(e))
        } else {
            outln(&e.line)
        }
    })
    .await?;
    // A signal ends the follow quietly with exit 0, like Go.
    Ok(())
}

pub async fn ports(g: &Globals) -> Result<()> {
    let client = daemon_client().await?;
    let res = client.ports().await?;
    if g.json {
        print_json(&res);
        return Ok(());
    }
    let mut t = Table::new();
    t.row([
        "PORT", "PROJECT", "SERVICE", "NAME", "STATE", "OWNER", "PID",
    ]);
    for p in &res.ports {
        t.row([
            p.lease.port.to_string(),
            p.lease.project.clone(),
            p.lease.service.clone(),
            p.lease.port_name.clone(),
            or_dash(p.state.map_or("", |s| s.as_str())),
            or_dash(&p.owner),
            pid_str(p.pid),
        ]);
    }
    out(&t.render());
    Ok(())
}

pub async fn gc(g: &Globals) -> Result<()> {
    let client = daemon_client().await?;
    let res = client.gc().await?;
    if g.json {
        print_json(&res);
        return Ok(());
    }
    if res.actions.is_empty() {
        outln("nothing to clean");
        return Ok(());
    }
    let mut t = Table::new();
    t.row(["ACTION", "PROJECT", "SERVICE", "DETAIL"]);
    for a in &res.actions {
        let mut detail = a.detail.clone();
        if a.port != 0 {
            detail = format!("port {} {}", a.port, a.detail).trim().to_owned();
        }
        t.row([
            a.action.clone(),
            a.project.clone(),
            a.service.clone(),
            or_dash(&detail),
        ]);
    }
    out(&t.render());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket_domain::api::ServiceResult;
    use rocket_domain::{PortRemap, RunState};
    use std::collections::BTreeMap;

    fn result() -> UpResult {
        UpResult {
            project: "fixture".into(),
            env: "dev".into(),
            services: vec![
                ServiceResult {
                    service: "api".into(),
                    action: "started".into(),
                    state: RunState::Running,
                    health: None,
                    pid: 7,
                    ports: BTreeMap::from([("http".to_owned(), 18531)]),
                    remaps: vec![PortRemap {
                        name: "http".into(),
                        from: 18431,
                        to: 18531,
                        env: "PORT".into(),
                        holder: None,
                        reason: "busy".into(),
                    }],
                    owner: String::new(),
                    expires_at: None,
                    log_path: String::new(),
                    error: String::new(),
                },
                ServiceResult {
                    service: "worker".into(),
                    action: "failed".into(),
                    state: RunState::Failed,
                    health: None,
                    pid: 0,
                    ports: BTreeMap::new(),
                    remaps: vec![],
                    owner: String::new(),
                    expires_at: None,
                    log_path: String::new(),
                    error: "exited early".into(),
                },
            ],
            hints: vec![
                "Run rocket migrate.".into(),
                "Run rocket setup assets.".into(),
            ],
        }
    }

    #[test]
    fn write_up_prints_hints() {
        let text = render_up(&UpResult {
            project: "fixture".into(),
            env: "dev".into(),
            services: vec![],
            hints: vec![
                "Run rocket migrate.".into(),
                "Run rocket setup assets.".into(),
            ],
        });
        for want in [
            "project fixture (env dev)",
            "hint: Run rocket migrate.",
            "hint: Run rocket setup assets.",
        ] {
            assert!(text.contains(want), "human output missing {want:?}: {text}");
        }
    }

    #[test]
    fn up_table_matches_the_go_layout() {
        assert_eq!(
            render_up(&result()),
            "project fixture (env dev)\n\
             SERVICE  ACTION   STATE    PORTS       NOTE\n\
             api      started  running  http=18531  http 18431->18531 via PORT (busy)\n\
             worker   failed   failed   -           exited early\n\
             hint: Run rocket migrate.\n\
             hint: Run rocket setup assets.\n"
        );
    }

    fn down_args(all: bool) -> DownArgs {
        DownArgs {
            services: vec![],
            all,
            owner: String::new(),
            everywhere: false,
        }
    }

    #[test]
    fn down_all_outside_a_project_suggests_everywhere() {
        let dir = tempfile::tempdir().unwrap();
        // Discovered from the cwd: guidance included.
        let err = down_request(&Globals::default(), dir.path(), down_args(true)).unwrap_err();
        assert!(err.to_string().contains("no rocket.yaml found"), "{err}");
        assert!(err.to_string().contains("--everywhere"), "{err}");

        // An explicit bad path: no guidance.
        let g = Globals {
            json: false,
            project: format!("{}/missing", dir.path().display()),
        };
        let err = down_request(&g, dir.path(), down_args(true)).unwrap_err();
        assert!(err.to_string().contains("no rocket.yaml found"), "{err}");
        assert!(!err.to_string().contains("--everywhere"), "{err}");
    }

    #[test]
    fn down_requires_a_selection_and_skips_the_project_for_everywhere() {
        let dir = tempfile::tempdir().unwrap();
        let err = down_request(&Globals::default(), dir.path(), down_args(false)).unwrap_err();
        assert!(err.to_string().starts_with("nothing selected"), "{err}");
        let req = down_request(
            &Globals::default(),
            dir.path(),
            DownArgs {
                services: vec![],
                all: false,
                owner: String::new(),
                everywhere: true,
            },
        )
        .unwrap();
        assert!(req.everywhere && req.project.is_empty());
    }
}

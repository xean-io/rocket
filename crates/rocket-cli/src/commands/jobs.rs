//! run, setup, doctor/install/migrate, deploy, jobs and job (Go: `jobs.go`).

use crate::cli::{DeployArgs, JobFlags, JobSelection, JobsArgs, RunArgs, SetupArgs, StepArgs};
use crate::commands::services::optional_project;
use crate::error::{CliError, EXIT_PARTIAL, Result};
use crate::format::{
    duration_str, errln, exit_str, go_duration, json_line, local_stamp, out, outln, print_json,
};
use crate::globals::{Globals, daemon_client, owner};
use crate::stream::{Followed, Interrupt, follow};
use crate::tabwriter::Table;
use rocket_client::JobsQuery;
use rocket_domain::api::JobRequest;
use rocket_domain::{Job, JobKind, JobStatus, event_type};

/// How many log lines a finished blocking job returns.
const TAIL_LINES: u32 = 50;

/// Starts a job and, unless detached, streams its log (human) or waits for
/// it (`--json`). Ctrl-C cancels the job. Failed jobs exit 2.
async fn run_job(
    g: &Globals,
    mut req: JobRequest,
    flags: JobFlags,
    selection: Option<JobSelection>,
) -> Result<()> {
    req.project = g.project_ref()?;
    req.owner = owner(&flags.owner);
    req.ttl = flags.ttl;
    if let Some(s) = selection {
        req.env = s.env;
        req.profiles = s.profiles;
    }
    let client = daemon_client().await?;
    let job = client.start_job(&req).await?;
    if flags.detach {
        if g.json {
            print_json(&job);
        } else {
            outln(&job.id);
            errln(&format!(
                "rocket: started job {} ({} {}); follow with `rocket job {} logs -f`",
                job.id, job.kind, job.name, job.id
            ));
        }
        return Ok(());
    }

    let mut interrupt = Interrupt::new()?;
    let waited: Result<Followed> = if g.json {
        tokio::select! {
            () = interrupt.wait() => Ok(Followed::Interrupted),
            r = client.job(&job.id, true) => r.map(|_| Followed::Ended).map_err(CliError::from),
        }
    } else {
        let stream = client.follow_job_logs(&job.id, None).await;
        match stream {
            Ok(stream) => {
                follow(stream, &mut interrupt, |e| {
                    if e.r#type == event_type::JOB_LOG {
                        outln(&e.line)
                    } else {
                        true
                    }
                })
                .await
            }
            Err(e) => Err(e.into()),
        }
    };
    match waited {
        Ok(Followed::Interrupted) => {
            errln(&format!("rocket: interrupted, canceling job {}", job.id));
            client.cancel_job(&job.id).await?;
        }
        Ok(_) => {}
        Err(e) => return Err(e),
    }
    let final_job = client.job(&job.id, true).await?;
    let logs = client
        .job_logs(&job.id, TAIL_LINES)
        .await
        .unwrap_or_default();
    let outcome = rocket_app::jobs::outcome(&final_job, logs.lines);
    if g.json {
        print_json(&outcome);
    } else {
        errln(&format!(
            "rocket: job {} ({} {}) {} in {}, exit {}",
            outcome.job,
            outcome.kind,
            outcome.name,
            outcome.status,
            go_duration(i128::from(outcome.duration_ms) * 1_000_000),
            exit_str(outcome.exit_code)
        ));
        if !outcome.error.is_empty() {
            errln(&format!("rocket: {}", outcome.error));
        }
        errln(&format!("rocket: log {}", outcome.log_path));
    }
    if final_job.status != JobStatus::Succeeded {
        return Err(CliError::Exit(EXIT_PARTIAL));
    }
    Ok(())
}

pub async fn run(g: &Globals, a: RunArgs) -> Result<()> {
    let mut args = a.args.into_iter();
    let name = args.next().unwrap_or_default();
    let req = JobRequest {
        kind: JobKind::Pipeline,
        name,
        args: args.collect(),
        ..JobRequest::default()
    };
    run_job(g, req, a.flags, Some(a.selection)).await
}

pub async fn setup(g: &Globals, a: SetupArgs) -> Result<()> {
    let req = JobRequest {
        kind: JobKind::Setup,
        name: a.name.unwrap_or_default(),
        ..JobRequest::default()
    };
    run_job(g, req, a.flags, Some(a.selection)).await
}

/// `rocket doctor|install|migrate`.
pub async fn setup_step(g: &Globals, name: &str, a: StepArgs) -> Result<()> {
    let req = JobRequest {
        kind: JobKind::Setup,
        name: name.to_owned(),
        ..JobRequest::default()
    };
    run_job(g, req, a.flags, Some(a.selection)).await
}

pub async fn deploy(g: &Globals, a: DeployArgs) -> Result<()> {
    let req = JobRequest {
        kind: JobKind::Deploy,
        name: a.env,
        yes: a.yes,
        allow_agent_deploy: std::env::var("ROCKET_ALLOW_DEPLOY").is_ok_and(|v| v == "1"),
        ..JobRequest::default()
    };
    run_job(g, req, a.flags, None).await
}

pub async fn list(g: &Globals, a: JobsArgs) -> Result<()> {
    let project = if a.all_projects {
        None
    } else {
        optional_project(g)?
    };
    let client = daemon_client().await?;
    let res = client
        .jobs(&JobsQuery {
            project,
            all: a.all_projects,
            limit: Some(a.limit),
        })
        .await?;
    if g.json {
        print_json(&res);
        return Ok(());
    }
    let mut t = Table::new();
    t.row([
        "ID", "PROJECT", "KIND", "NAME", "STATUS", "EXIT", "DURATION", "OWNER", "STARTED",
    ]);
    for j in &res.jobs {
        t.row([
            j.id.clone(),
            j.project.clone(),
            j.kind.to_string(),
            j.name.clone(),
            j.status.to_string(),
            exit_str(j.exit_code),
            duration_str(j.duration_ms),
            j.owner.clone(),
            local_stamp(j.started_at),
        ]);
    }
    out(&t.render());
    Ok(())
}

/// `rocket job <id> [logs]`.
pub async fn show(
    g: &Globals,
    args: Vec<String>,
    follow_flag: bool,
    tail: Option<u32>,
) -> Result<()> {
    if args.len() == 2 && args[1] != "logs" {
        return Err(CliError::msg(format!(
            "unknown job action {:?} (use `rocket job <id> logs` or `rocket job cancel <id>`)",
            args[1]
        )));
    }
    let client = daemon_client().await?;
    let id = &args[0];
    if args.len() == 1 {
        let job = client.job(id, false).await?;
        if g.json {
            print_json(&job);
        } else {
            out(&render_job(&job));
        }
        return Ok(());
    }
    if !follow_flag {
        let res = client.job_logs(id, tail.unwrap_or(100)).await?;
        if g.json {
            print_json(&res);
        } else {
            for l in &res.lines {
                outln(l);
            }
        }
        return Ok(());
    }
    let stream = client.follow_job_logs(id, tail).await?;
    let mut interrupt = Interrupt::new()?;
    let json = g.json;
    follow(stream, &mut interrupt, |e| {
        if json {
            outln(&json_line(e))
        } else if e.r#type == event_type::JOB_LOG {
            outln(&e.line)
        } else {
            true
        }
    })
    .await?;
    Ok(())
}

pub async fn cancel(g: &Globals, id: &str) -> Result<()> {
    let client = daemon_client().await?;
    let job = client.cancel_job(id).await?;
    if g.json {
        print_json(&job);
    } else {
        outln(&format!("job {} {}", job.id, job.status));
    }
    Ok(())
}

/// The key/value block of `rocket job <id>` (Go: `printJob`).
pub fn render_job(j: &Job) -> String {
    let mut t = Table::new();
    t.row(["id".to_owned(), j.id.clone()]);
    t.row(["project".to_owned(), j.project.clone()]);
    t.row(["kind/name".to_owned(), format!("{} {}", j.kind, j.name)]);
    t.row([
        "status".to_owned(),
        format!("{} (exit {})", j.status, exit_str(j.exit_code)),
    ]);
    t.row(["owner".to_owned(), j.owner.clone()]);
    t.row(["duration".to_owned(), duration_str(j.duration_ms)]);
    for (i, s) in j.steps.iter().enumerate() {
        let mark = if i32::try_from(i + 1).is_ok_and(|n| n == j.step) {
            ">"
        } else {
            " "
        };
        t.row([
            format!("step {}", i + 1),
            format!("{mark} {}", s.describe()),
        ]);
    }
    if !j.args.is_empty() {
        t.row(["args".to_owned(), j.args.join(" ")]);
    }
    if !j.error.is_empty() {
        t.row(["error".to_owned(), j.error.clone()]);
    }
    t.row(["log".to_owned(), j.log_path.clone()]);
    t.render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket_domain::Step;
    use time::OffsetDateTime;

    fn job() -> Job {
        Job {
            id: "j1".into(),
            project: "fixture".into(),
            name: "check".into(),
            kind: JobKind::Pipeline,
            env: String::new(),
            profiles: vec![],
            owner: "user".into(),
            steps: vec![
                Step {
                    task: String::new(),
                    run: "echo one".into(),
                },
                Step {
                    task: "build".into(),
                    run: String::new(),
                },
            ],
            args: vec!["a".into(), "b".into()],
            status: JobStatus::Failed,
            step: 2,
            pid: 0,
            pgid: 0,
            exit_code: Some(3),
            started_at: OffsetDateTime::UNIX_EPOCH,
            expires_at: None,
            finished_at: None,
            duration_ms: 1_234,
            log_path: "/x/job.log".into(),
            error: "step 2 failed".into(),
        }
    }

    #[test]
    fn renders_a_job_like_go() {
        assert_eq!(
            render_job(&job()),
            "id         j1\n\
             project    fixture\n\
             kind/name  pipeline check\n\
             status     failed (exit 3)\n\
             owner      user\n\
             duration   1.2s\n\
             step 1       run echo one\n\
             step 2     > task build\n\
             args       a b\n\
             error      step 2 failed\n\
             log        /x/job.log\n"
        );
    }
}

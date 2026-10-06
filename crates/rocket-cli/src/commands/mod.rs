//! Command implementations, one module per Go source file.

mod admin;
mod ai;
mod jobs;
mod services;

pub use admin::DaemonStatus;
pub use ai::{AgentInstallResult, InitResult};
pub use services::{down_request, render_up};

use crate::cli::{AgentCommand, Cli, Command, DaemonCommand, JobCommand, ProjectsCommand};
use crate::error::Result;
use crate::globals::Globals;

/// Runs the parsed command line.
pub async fn dispatch(cli: Cli) -> Result<()> {
    let g = Globals {
        json: cli.json,
        project: cli.project,
    };
    let Some(command) = cli.command else {
        return help(&[]);
    };
    match command {
        Command::Up(a) => services::up(&g, a.into()).await,
        Command::Restart(a) => services::up(&g, a.into()).await,
        Command::Down(a) => services::down(&g, a).await,
        Command::Ps(a) => services::ps(&g, a).await,
        Command::Logs(a) => services::logs(&g, a).await,
        Command::Ports => services::ports(&g).await,
        Command::Gc => services::gc(&g).await,
        Command::Projects { command } => match command {
            None => help(&["projects"]),
            Some(ProjectsCommand::Add { path }) => admin::projects_add(g, path).await,
            Some(ProjectsCommand::Ls) => admin::projects_ls(&g).await,
            Some(ProjectsCommand::Rm { name }) => admin::projects_rm(&g, &name).await,
        },
        Command::Daemon { command } => match command {
            None => help(&["daemon"]),
            Some(DaemonCommand::Run) => admin::daemon_run().await,
            Some(DaemonCommand::Start) => admin::daemon_start(&g).await,
            Some(DaemonCommand::Stop) => admin::daemon_stop(&g).await,
            Some(DaemonCommand::Status) => admin::daemon_status(&g).await,
        },
        Command::Schema => admin::schema(),
        Command::Run(a) => jobs::run(&g, a).await,
        Command::Setup(a) => jobs::setup(&g, a).await,
        Command::Doctor(a) => jobs::setup_step(&g, "doctor", a).await,
        Command::Install(a) => jobs::setup_step(&g, "install", a).await,
        Command::Migrate(a) => jobs::setup_step(&g, "migrate", a).await,
        Command::Deploy(a) => jobs::deploy(&g, a).await,
        Command::Jobs(a) => jobs::list(&g, a).await,
        Command::Job(a) => match a.command {
            Some(JobCommand::Cancel { id }) => jobs::cancel(&g, &id).await,
            None => jobs::show(&g, a.args, a.follow, a.tail).await,
        },
        Command::Status => ai::status(&g).await,
        Command::Init(a) => ai::init(&g, a),
        Command::Agent { command } => match command {
            None => help(&["agent"]),
            Some(AgentCommand::Install {
                target,
                global,
                print_only,
            }) => ai::agent_install(&g, &target, global, print_only),
        },
        Command::App => ai::app(&g).await,
    }
}

/// Prints the help of the command at `path` (cobra does that for a group
/// command invoked without a subcommand) and succeeds.
fn help(path: &[&str]) -> Result<()> {
    crate::print_help(path);
    Ok(())
}

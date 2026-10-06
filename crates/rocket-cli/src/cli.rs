//! The command tree: names, aliases, flags and help text of the Go cobra
//! CLI (`cmd/rocket`), declared with clap.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "rocket",
    about = "Single owner of every dev process across your projects",
    // cobra's `-v, --version` is added in `parse::command()`.
    disable_version_flag = true,
)]
pub struct Cli {
    /// emit machine-readable JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// project name or path (default: rocket.yaml found from cwd)
    #[arg(
        short = 'p',
        long = "project",
        global = true,
        default_value = "",
        hide_default_value = true,
        value_name = "PROJECT"
    )]
    pub project: String,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start services (and their dependencies); idempotent
    Up(UpArgs),
    /// Stop services (reverse dependency order)
    Down(DownArgs),
    /// Stop then start services
    Restart(RestartArgs),
    /// Show service state
    Ps(PsArgs),
    /// Show service output
    Logs(LogsArgs),
    /// Show the global port map across projects
    Ports,
    /// Reconcile state, expire TTLs, release stale leases, prune old runs
    Gc,
    /// Manage the global project registry
    Projects {
        #[command(subcommand)]
        command: Option<ProjectsCommand>,
    },
    /// Manage rocketd
    Daemon {
        #[command(subcommand)]
        command: Option<DaemonCommand>,
    },
    /// Print the JSON Schema of rocket.yaml
    Schema,
    /// Run a pipeline (or a Taskfile task) as a supervised job
    #[command(
        long_about = "Run a pipeline (or a Taskfile task) as a supervised job\n\n\
        Runs pipelines.<name> from rocket.yaml; unknown names fall back to `task <name>` of the project's Taskfile.\n\
        Steps run sequentially and stop at the first failure. Arguments after -- go to the last step.\n\
        Blocks until done (exit 0 on success, 2 on failure); --detach returns the job id."
    )]
    Run(RunArgs),
    /// Run setup.doctor → setup.install → setup.migrate (skipping undefined), or one setup.<name>
    Setup(SetupArgs),
    /// Run setup.doctor from rocket.yaml
    Doctor(StepArgs),
    /// Run setup.install from rocket.yaml
    Install(StepArgs),
    /// Run setup.migrate from rocket.yaml
    Migrate(StepArgs),
    /// Run envs.<env>.deploy (needs --yes when confirm: true)
    #[command(
        long_about = "Run envs.<env>.deploy (needs --yes when confirm: true)\n\n\
        Envs with `confirm: true` require --yes. Agents (owner agent:*) are refused unless --yes is passed AND\n\
        ROCKET_ALLOW_DEPLOY=1 is set by a human. A refused deploy exits 3."
    )]
    Deploy(DeployArgs),
    /// List recent jobs (newest first)
    Jobs(JobsArgs),
    /// Show a job, or its log with `logs` (-f to follow)
    Job(JobArgs),
    /// One-shot summary for agents: project, services, running jobs, port conflicts
    Status,
    /// Detect Taskfile/compose/.env and scaffold rocket.yaml
    #[command(
        long_about = "Detect Taskfile/compose/.env and scaffold rocket.yaml\n\n\
        Scans the current directory (or -p <dir>) for Taskfile.yml, compose files and .env and writes a\n\
        starting rocket.yaml. Refuses to overwrite an existing one unless --force; --print writes nothing."
    )]
    Init(InitArgs),
    /// Teach AI agents to use rocket
    Agent {
        #[command(subcommand)]
        command: Option<AgentCommand>,
    },
    /// Open Rocket.app (macOS); starts the daemon first
    App,
}

#[derive(Debug, Args)]
pub struct UpArgs {
    /// services or groups to start (default: all)
    #[arg(value_name = "SERVICE|GROUP")]
    pub services: Vec<String>,
    /// environment (default: project default_env)
    #[arg(long, default_value = "")]
    pub env: String,
    /// enable a startup profile (repeatable)
    #[arg(long = "profile", action = clap::ArgAction::Append)]
    pub profiles: Vec<String>,
    /// owner tag (default: $ROCKET_OWNER or "user")
    #[arg(long, default_value = "")]
    pub owner: String,
    /// stop automatically after this duration, e.g. 30m
    #[arg(long, default_value = "")]
    pub ttl: String,
}

#[derive(Debug, Args)]
pub struct RestartArgs {
    /// services or groups to restart
    #[arg(value_name = "SERVICE|GROUP", required = true)]
    pub services: Vec<String>,
    /// environment (default: project default_env)
    #[arg(long, default_value = "")]
    pub env: String,
    /// enable a startup profile (repeatable)
    #[arg(long = "profile", action = clap::ArgAction::Append)]
    pub profiles: Vec<String>,
    /// owner tag (default: $ROCKET_OWNER or "user")
    #[arg(long, default_value = "")]
    pub owner: String,
    /// stop automatically after this duration, e.g. 30m
    #[arg(long, default_value = "")]
    pub ttl: String,
}

#[derive(Debug, Args)]
pub struct DownArgs {
    /// services or groups to stop
    #[arg(value_name = "SERVICE|GROUP")]
    pub services: Vec<String>,
    /// stop every service of the project
    #[arg(long)]
    pub all: bool,
    /// only stop services started by this owner
    #[arg(long, default_value = "")]
    pub owner: String,
    /// apply to all projects
    #[arg(long)]
    pub everywhere: bool,
}

#[derive(Debug, Args)]
pub struct PsArgs {
    /// show every project
    #[arg(long = "all-projects")]
    pub all_projects: bool,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// the service to show
    pub service: String,
    /// stream new lines
    #[arg(short = 'f', long)]
    pub follow: bool,
    /// number of lines to show
    #[arg(long, default_value_t = 100)]
    pub tail: u32,
}

#[derive(Debug, Subcommand)]
pub enum ProjectsCommand {
    /// Register a project (default: rocket.yaml found from cwd)
    Add {
        /// project directory
        path: Option<String>,
    },
    /// List registered projects
    #[command(visible_alias = "list")]
    Ls,
    /// Unregister a project (must have nothing running)
    #[command(visible_alias = "remove")]
    Rm {
        /// registered project name
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// Run the daemon in the foreground
    Run,
    /// Start the daemon in the background (no-op when running)
    Start,
    /// Stop the daemon (supervised services keep running and are adopted on next start)
    Stop,
    /// Show daemon status (exit 1 when not running)
    Status,
}

/// Flags shared by `run`, `setup` and the setup steps.
#[derive(Debug, Args)]
pub struct JobFlags {
    /// owner tag (default: $ROCKET_OWNER or "user")
    #[arg(long, default_value = "")]
    pub owner: String,
    /// cancel automatically after this duration, e.g. 30m
    #[arg(long, default_value = "")]
    pub ttl: String,
    /// return the job id immediately instead of waiting
    #[arg(long)]
    pub detach: bool,
}

/// `--env` and `--profile`: not offered by `deploy`.
#[derive(Debug, Args)]
pub struct JobSelection {
    /// environment (default: project default_env)
    #[arg(long, default_value = "")]
    pub env: String,
    /// enable a prerequisite startup profile (repeatable)
    #[arg(long = "profile", action = clap::ArgAction::Append)]
    pub profiles: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// pipeline or task name, then arguments for the last step
    #[arg(value_name = "PIPELINE|TASK", num_args = 1.., required = true)]
    pub args: Vec<String>,
    #[command(flatten)]
    pub flags: JobFlags,
    #[command(flatten)]
    pub selection: JobSelection,
}

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// run only setup.<name>
    pub name: Option<String>,
    #[command(flatten)]
    pub flags: JobFlags,
    #[command(flatten)]
    pub selection: JobSelection,
}

#[derive(Debug, Args)]
pub struct StepArgs {
    #[command(flatten)]
    pub flags: JobFlags,
    #[command(flatten)]
    pub selection: JobSelection,
}

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// the environment to deploy
    pub env: String,
    #[command(flatten)]
    pub flags: JobFlags,
    /// confirm the deploy
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct JobsArgs {
    /// show jobs of every project
    #[arg(long = "all-projects")]
    pub all_projects: bool,
    /// maximum number of jobs
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct JobArgs {
    /// the job id, optionally followed by `logs`
    #[arg(value_name = "ID [logs]", num_args = 1..=2, required = true)]
    pub args: Vec<String>,
    /// stream the log until the job ends
    #[arg(short = 'f', long)]
    pub follow: bool,
    /// number of lines (with -f: backlog lines; default whole log)
    #[arg(long, value_name = "N")]
    pub tail: Option<u32>,
    #[command(subcommand)]
    pub command: Option<JobCommand>,
}

#[derive(Debug, Subcommand)]
pub enum JobCommand {
    /// Cancel a running job (stops its process group)
    Cancel {
        /// the job id
        id: String,
    },
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// overwrite an existing rocket.yaml
    #[arg(long)]
    pub force: bool,
    /// print the generated rocket.yaml instead of writing it
    #[arg(long = "print")]
    pub print_only: bool,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Write the rocket skill and an AGENTS.md/CLAUDE.md block (idempotent)
    #[command(
        long_about = "Write the rocket skill and an AGENTS.md/CLAUDE.md block (idempotent)\n\n\
        --target claude: Claude Code skill + CLAUDE.md block; agents: AGENTS.md block; both (default): all.\n\
        The skill goes to .claude/skills/rocket/SKILL.md in the project, or ~/.claude/skills with --global.\n\
        Blocks go to the project root (rocket.yaml found from cwd, else cwd; skipped with --global outside a project)."
    )]
    Install {
        /// claude | agents | both
        #[arg(long, default_value = "both")]
        target: String,
        /// install the skill under ~/.claude/skills instead of the project
        #[arg(long)]
        global: bool,
        /// show the content without writing
        #[arg(long = "print")]
        print_only: bool,
    },
}

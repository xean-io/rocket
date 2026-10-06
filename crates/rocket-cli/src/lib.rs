//! The `rocket` command line (ported from the original Go CLI): a port of the cobra tree
//! with the same commands, flags, human and `--json` output and exit codes
//! (0 ok, 1 error, 2 partial failure or failed job, 3 confirmation
//! required). `rocket daemon run` runs the daemon in-process.
//!
//! Differences from the Go binary: usage errors come from clap (same
//! `rocket: <message>` framing and exit code 1, different wording), there is
//! and there is no generated `completion` command. `--version` prints
//! `rocket version <version>` like cobra.

pub mod cli;
pub mod commands;
pub mod error;
pub mod format;
pub mod globals;
pub mod stream;
pub mod tabwriter;
pub mod version;

use clap::error::ErrorKind;
use clap::{Arg, ArgAction, CommandFactory, FromArgMatches};
use cli::Cli;
use error::{CliError, EXIT_ERROR};

/// The clap command with cobra's `-v, --version` flag.
pub fn command() -> clap::Command {
    // clap prints `<name> <version>`; the prefix reproduces cobra's
    // `rocket version <version>`.
    let shown: &'static str = Box::leak(format!("version {}", version::version()).into_boxed_str());
    Cli::command().version(shown).arg(
        Arg::new("version")
            .short('v')
            .long("version")
            .action(ArgAction::Version)
            .help("version for rocket"),
    )
}

/// Prints the help of the subcommand at `path` (empty: the root command).
pub fn print_help(path: &[&str]) {
    let mut cmd = command();
    cmd.build();
    let mut cur = &cmd;
    for name in path {
        match cur.find_subcommand(name) {
            Some(sub) => cur = sub,
            None => break,
        }
    }
    let _ = cur.clone().print_help();
}

/// `rocket run -- foo`: cobra rejects a command line whose arguments all
/// come after `--` (`ArgsLenAtDash() == 0`). Scans `argv` (program name
/// first) for `run` followed by `--` before any positional argument.
fn run_dash_first(argv: &[String]) -> bool {
    // Flags that consume the next token, globally and for `run`.
    const GLOBAL_VALUE_FLAGS: [&str; 2] = ["-p", "--project"];
    const RUN_VALUE_FLAGS: [&str; 4] = ["--owner", "--ttl", "--env", "--profile"];
    let mut tokens = argv.iter().skip(1);
    let mut in_run = false;
    while let Some(tok) = tokens.next() {
        if tok == "--" {
            return in_run;
        }
        let takes_value = GLOBAL_VALUE_FLAGS.contains(&tok.as_str())
            || (in_run && RUN_VALUE_FLAGS.contains(&tok.as_str()));
        if takes_value {
            tokens.next();
        } else if tok.starts_with('-') && tok.len() > 1 {
            // A boolean flag or `--flag=value`.
        } else if in_run {
            return false; // a positional came first
        } else if tok == "run" {
            in_run = true;
        } else {
            return false; // another subcommand
        }
    }
    false
}

/// Parses `argv` (including the program name).
pub fn parse(argv: &[String]) -> Result<Cli, CliError> {
    let matches = command().try_get_matches_from(argv).map_err(usage_error)?;
    if run_dash_first(argv) {
        return Err(CliError::msg("pipeline name must come before --"));
    }
    Cli::from_arg_matches(&matches).map_err(usage_error)
}

/// A usage error as the first paragraph of clap's message, without the
/// `error: ` prefix.
fn usage_error(e: clap::Error) -> CliError {
    let rendered = e.render().to_string();
    let first = rendered.split("\n\n").next().unwrap_or("").trim();
    CliError::msg(first.strip_prefix("error: ").unwrap_or(first))
}

/// Runs the CLI and returns the process exit code.
pub fn run_process(argv: Vec<String>) -> i32 {
    let wants_json = argv
        .iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|a| a == "--json");
    let cli = match command().try_get_matches_from(&argv) {
        Ok(_) => match parse(&argv) {
            Ok(cli) => cli,
            Err(e) => return report(&e, wants_json),
        },
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            let _ = e.print();
            return 0;
        }
        Err(e) => return report(&usage_error(e), wants_json),
    };
    let json = cli.json;
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return report(&CliError::msg(format!("start runtime: {e}")), json),
    };
    match runtime.block_on(commands::dispatch(cli)) {
        Ok(()) => 0,
        Err(e) => report(&e, json),
    }
}

/// Prints a failure the way Go's `main` does and returns the exit code.
fn report(err: &CliError, json: bool) -> i32 {
    let (code, error_code) = err.codes();
    let Some(text) = err.text() else {
        return code;
    };
    if json {
        format::out(&format::json_error(&text, &error_code));
    } else {
        format::errln(&format!("rocket: {text}"));
    }
    if code == 0 { EXIT_ERROR } else { code }
}

/// Entry point of the `rocket` binary.
pub fn main() -> ! {
    // The `time` crate reads the local offset soundly only before any
    // thread exists; both the table timestamps and the daemon log need it.
    format::init_local_offset();
    rocket_daemon::init_local_offset();
    let argv: Vec<String> = std::env::args().collect();
    std::process::exit(run_process(argv))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        std::iter::once("rocket")
            .chain(args.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn dash_before_the_pipeline_name_is_detected() {
        assert!(run_dash_first(&argv(&["run", "--", "foo"])));
        assert!(run_dash_first(&argv(&[
            "-p", "x", "run", "--json", "--", "foo"
        ])));
        assert!(run_dash_first(&argv(&[
            "run", "--owner", "me", "--", "foo"
        ])));
        assert!(!run_dash_first(&argv(&["run", "check", "--", "a"])));
        assert!(!run_dash_first(&argv(&[
            "run", "--ttl", "1s", "check", "--", "a"
        ])));
        assert!(!run_dash_first(&argv(&["up", "--", "a"])));
        assert!(!run_dash_first(&argv(&["run", "check"])));
    }

    #[test]
    fn args_after_dash_reach_the_run_command() {
        let cli = parse(&argv(&["run", "echo-args", "--json", "--", "a", "b c"])).unwrap();
        assert!(cli.json);
        let Some(cli::Command::Run(r)) = cli.command else {
            panic!("not run")
        };
        assert_eq!(r.args, ["echo-args", "a", "b c"]);
        let err = parse(&argv(&["run", "--", "foo"])).unwrap_err();
        assert_eq!(err.to_string(), "pipeline name must come before --");
    }

    #[test]
    fn repeated_flags_and_positionals() {
        let cli = parse(&argv(&[
            "up",
            "all",
            "*",
            "--profile",
            "a",
            "--profile",
            "a",
            "--ttl",
            "5m",
            "-p",
            "x",
        ]))
        .unwrap();
        assert_eq!(cli.project, "x");
        let Some(cli::Command::Up(u)) = cli.command else {
            panic!("not up")
        };
        assert_eq!(u.services, ["all", "*"]);
        assert_eq!(u.profiles, ["a", "a"]);
        assert_eq!(u.ttl, "5m");
    }

    #[test]
    fn job_tail_distinguishes_unset_from_set() {
        let Some(cli::Command::Job(j)) =
            parse(&argv(&["job", "id1", "logs", "-f"])).unwrap().command
        else {
            panic!()
        };
        assert_eq!(
            (j.args, j.follow, j.tail),
            (vec!["id1".into(), "logs".into()], true, None)
        );
        let Some(cli::Command::Job(j)) = parse(&argv(&["job", "id1", "logs", "-f", "--tail", "0"]))
            .unwrap()
            .command
        else {
            panic!()
        };
        assert_eq!(j.tail, Some(0));
        let Some(cli::Command::Job(j)) = parse(&argv(&["job", "cancel", "id1"])).unwrap().command
        else {
            panic!()
        };
        assert!(matches!(j.command, Some(cli::JobCommand::Cancel { id }) if id == "id1"));
    }

    #[test]
    fn aliases_and_unknown_flags() {
        assert!(parse(&argv(&["projects", "list"])).is_ok());
        assert!(parse(&argv(&["projects", "remove", "x"])).is_ok());
        for flag in ["--profile", "--env"] {
            assert!(
                parse(&argv(&["deploy", "stage", "--yes", flag, "extra"])).is_err(),
                "{flag}"
            );
        }
        assert!(parse(&argv(&["down", "--bogus"])).is_err());
    }

    #[test]
    fn go_default_values() {
        let Some(cli::Command::Logs(l)) = parse(&argv(&["logs", "web"])).unwrap().command else {
            panic!()
        };
        assert_eq!(l.tail, 100);
        let Some(cli::Command::Jobs(j)) = parse(&argv(&["jobs"])).unwrap().command else {
            panic!()
        };
        assert_eq!(j.limit, 50);
        let Some(cli::Command::Agent {
            command: Some(cli::AgentCommand::Install { target, .. }),
        }) = parse(&argv(&["agent", "install"])).unwrap().command
        else {
            panic!()
        };
        assert_eq!(target, "both");
    }
}

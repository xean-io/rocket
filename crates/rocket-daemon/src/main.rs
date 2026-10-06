//! `rocketd`: runs the daemon in the foreground (the `rocket` CLI embeds the
//! same [`rocket_daemon::run`] as `rocket daemon run`). Honors `ROCKET_HOME`.

fn main() {
    rocket_daemon::init_local_offset(); // before any thread exists
    let result = tokio::runtime::Runtime::new()
        .map_err(anyhow::Error::from)
        .and_then(|rt| {
            let paths = rocket_client::Paths::resolve()?;
            rt.block_on(rocket_daemon::run(rocket_daemon::RunOptions::for_process(
                paths,
                env!("CARGO_PKG_VERSION"),
            )))
        });
    if let Err(err) = result {
        eprintln!("rocketd: {err}");
        std::process::exit(1);
    }
}

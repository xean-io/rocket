<!-- rocket:begin -->
## Processes, services, tests and CI: use rocket

- Use rocket for ALL process, service, test and CI operations. Never start processes yourself:
  no `&`, `nohup`, `docker compose up`, `task dev`, watchers or dev servers outside rocket.
- Identify yourself on every command: `export ROCKET_OWNER=agent:<session-id>` (or `--owner agent:<session-id>`).
- Always pass `--ttl` for exploratory runs: `rocket up api --ttl 30m`.
- Before finishing, clean up everything you started: `rocket down --owner agent:<session-id>`.
- Run targeted checks, not full suites: `rocket run <targeted pipeline|task> [-- args]`
  (e.g. `rocket run test:api -- -run TestLogin`).
- Always add `--json` and check exit codes: 0 ok, 1 error, 2 partial or job failure, 3 confirmation required.
- Look before acting: `rocket status --json` shows services, ports, running jobs and port conflicts.
- Deploys need a human: `rocket deploy <env>` is refused for agents unless a human passes `--yes`
  and sets `ROCKET_ALLOW_DEPLOY=1`.

```sh
export ROCKET_OWNER=agent:<session-id>
rocket status --json                      # project, services, ports, running jobs, conflicts
rocket up <svc|group> --ttl 30m --json    # idempotent; dependencies first; busy ports remapped
rocket ps --json | rocket ports --json
rocket logs <svc> --tail 100 --json
rocket run <pipeline|task> --json [-- args]   # blocks; {job,status,exit_code,duration_ms,log_path,tail}
rocket run <pipeline> --detach --json     # returns the job; then rocket job <id> logs -f
rocket jobs --json | rocket job <id> --json | rocket job cancel <id>
rocket setup --json                       # doctor -> install -> migrate
rocket down --owner agent:<session-id> --json
```
<!-- rocket:end -->

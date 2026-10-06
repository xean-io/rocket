// Typed wrappers over the Tauri commands exposed by src-tauri/src/commands.rs.
// Wire types come from the generated ./bindings.ts.
import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  CommandError,
  ConnectionStatus,
  DaemonDetails,
  DownRequest,
  DownResult,
  FollowMessage,
  GcResult,
  HealthInfo,
  Job,
  JobLogsResult,
  JobRequest,
  JobsResult,
  LogsResult,
  PortsResult,
  ProjectRef,
  ProjectsResult,
  RemovedProject,
  StatusResult,
  Summary,
  UpRequest,
  UpResult,
} from "./bindings";

/** Event names emitted by the backend supervisor. */
export const EVENTS = {
  /** Payload: `Event` (one daemon event). */
  event: "rocket://event",
  /** Payload: `ConnectionStatus`. */
  connection: "rocket://connection",
  /** No payload: reload every snapshot (the stream reopened). */
  resync: "rocket://resync",
} as const;

/** A failed command: `code` is the rocket error code or a local one. */
export class RocketError extends Error {
  readonly code: string;
  readonly status: number | null;

  constructor({ code, message, status }: CommandError) {
    super(message);
    this.name = "RocketError";
    this.code = code;
    this.status = status;
  }

  /** A deploy to a `confirm: true` env needs `yes: true` (HTTP 428). */
  get confirmationRequired(): boolean {
    return this.code === "confirmation_required";
  }
}

function isCommandError(e: unknown): e is CommandError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as CommandError).code === "string" &&
    typeof (e as CommandError).message === "string"
  );
}

/** Invokes a command, turning the serialized `CommandError` into a `RocketError`. */
export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    if (isCommandError(e)) throw new RocketError(e);
    throw e instanceof Error ? e : new Error(String(e));
  }
}

export const rocket = {
  connectionStatus: () => call<ConnectionStatus>("connection_status"),
  health: () => call<HealthInfo>("health"),
  projects: () => call<ProjectsResult>("projects"),
  addProject: (path: string) => call<ProjectRef>("add_project", { path }),
  removeProject: (name: string) => call<RemovedProject>("remove_project", { name }),
  up: (request: UpRequest) => call<UpResult>("up", { request }),
  restart: (request: UpRequest) => call<UpResult>("restart", { request }),
  down: (request: DownRequest) => call<DownResult>("down", { request }),
  ps: (project: string | null = null, all = false) =>
    call<StatusResult>("ps", { project, all }),
  status: (project: string | null = null) => call<Summary>("status", { project }),
  ports: () => call<PortsResult>("ports"),
  logs: (project: string, service: string, tail = 300) =>
    call<LogsResult>("logs", { project, service, tail }),
  gc: () => call<GcResult>("gc"),
  jobs: (opts: { project?: string | null; all?: boolean; limit?: number | null } = {}) =>
    call<JobsResult>("jobs", {
      project: opts.project ?? null,
      all: opts.all ?? false,
      limit: opts.limit ?? null,
    }),
  job: (id: string, wait = false) => call<Job>("job", { id, wait }),
  startJob: (request: JobRequest) => call<Job>("start_job", { request }),
  cancelJob: (id: string) => call<Job>("cancel_job", { id }),
  jobLogs: (id: string, tail = 300) => call<JobLogsResult>("job_logs", { id, tail }),
  daemonInfo: () => call<DaemonDetails>("daemon_info"),
  restartDaemon: () => call<DaemonDetails>("restart_daemon"),
  reconnect: () => call<void>("reconnect"),
};

/** Stops a log follow; safe to call more than once. */
export type StopFollow = () => Promise<void>;

async function follow(
  cmd: string,
  args: Record<string, unknown>,
  onMessage: (m: FollowMessage) => void,
): Promise<StopFollow> {
  const channel = new Channel<FollowMessage>();
  channel.onmessage = onMessage;
  const followId = await call<string>(cmd, { ...args, channel });
  let stopped = false;
  return async () => {
    if (stopped) return;
    stopped = true;
    await call<void>("stop_follow", { followId });
  };
}

/** Streams a job's log lines, then its final state. `tail: null` replays everything. */
export function followJobLogs(
  id: string,
  onMessage: (m: FollowMessage) => void,
  tail: number | null = null,
): Promise<StopFollow> {
  return follow("follow_job_logs", { id, tail }, onMessage);
}

/** Streams a service's log lines (tail first, then live). */
export function followServiceLogs(
  project: string,
  service: string,
  onMessage: (m: FollowMessage) => void,
  tail: number | null = 300,
): Promise<StopFollow> {
  return follow("follow_service_logs", { project, service, tail }, onMessage);
}

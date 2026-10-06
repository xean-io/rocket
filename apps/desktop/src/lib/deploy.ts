// Deploy confirmation flow: a deploy never runs without an explicit yes.
import type { Job, JobRequest } from "./bindings";
import { type DeployIntent, useNav } from "./nav";
import { RocketError, rocket } from "./rocket";

/** The request a confirmed deploy sends: always `yes: true`. */
export function deployRequest(intent: DeployIntent): JobRequest {
  const base: JobRequest = intent.request ?? {
    project: intent.project,
    kind: "deploy",
    name: intent.env,
    env: intent.env,
  };
  return { ...base, yes: true };
}

/** Sends the confirmed deploy (the dialog's Deploy button). */
export function confirmDeploy(intent: DeployIntent): Promise<Job> {
  return rocket.startJob(deployRequest(intent));
}

/**
 * Starts a job. When the daemon answers `confirmation_required` (HTTP 428) the
 * confirmation dialog opens for it and `null` is returned; other errors throw.
 */
export async function startJobOrConfirm(request: JobRequest): Promise<Job | null> {
  try {
    return await rocket.startJob(request);
  } catch (e) {
    if (e instanceof RocketError && e.confirmationRequired) {
      const env = request.env ?? request.name ?? "";
      useNav.getState().requestDeploy({ project: request.project, env, request });
      return null;
    }
    throw e;
  }
}

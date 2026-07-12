import type { SessionGitStatus } from "./ipc";

export function shouldShowGitIcon(
  status: SessionGitStatus | null | undefined,
): boolean {
  return status != null && status.dirty === true;
}

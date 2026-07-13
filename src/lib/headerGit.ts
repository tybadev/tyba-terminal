import type { SessionGitStatus } from "./ipc";

export type GitIconTone = "dirty" | "clean";

export function gitIconTone(
  status: SessionGitStatus | null | undefined,
): GitIconTone | null {
  if (status == null) return null;
  return status.dirty ? "dirty" : "clean";
}

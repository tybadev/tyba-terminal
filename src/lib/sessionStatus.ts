import type { SessionStatus } from "./ipc";

export const isFinishedStatus = (status: SessionStatus): boolean =>
  status.state === "exited" || status.state === "failed";

export const sameSessionStatus = (
  a: SessionStatus,
  b: SessionStatus,
): boolean => {
  if (a.state !== b.state) return false;
  switch (a.state) {
    case "awaiting_input":
      return a.hint === (b as typeof a).hint;
    case "exited":
      return a.code === (b as typeof a).code;
    case "failed":
      return a.reason === (b as typeof a).reason;
    default:
      return true;
  }
};

import type { SessionStatus } from "./ipc";

export const isFinishedStatus = (status: SessionStatus): boolean =>
  status.state === "exited" || status.state === "failed";

export const sameSessionStatus = (
  a: SessionStatus,
  b: SessionStatus,
): boolean => {
  if (a.state !== b.state) return false;
  switch (a.state) {
    case "awaiting_input": {
      const other = b as typeof a;
      return a.hint === other.hint && a.reason === other.reason;
    }
    case "idle":
      return a.summary === (b as typeof a).summary;
    case "exited":
      return a.code === (b as typeof a).code;
    case "failed":
      return a.reason === (b as typeof a).reason;
    default:
      return true;
  }
};

export interface StatusVisual {
  dotClass: string;
  textClass: string;
  labelKey: string;
  rank: number;
}

export const statusVisual = (
  status: SessionStatus,
  attention: boolean,
): StatusVisual | null => {
  switch (status.state) {
    case "failed":
      return {
        dotClass: "bg-tyba-red",
        textClass: "text-tyba-red",
        labelKey: "sessionFailed",
        rank: 4,
      };
    case "awaiting_input":
      return {
        dotClass:
          "bg-tyba-amber [box-shadow:var(--tyba-glow-amber)] motion-safe:animate-pulse",
        textClass: "text-tyba-amber",
        labelKey:
          status.reason === "approval"
            ? "sessionBlocked"
            : "sessionAwaiting",
        rank: 3,
      };
    case "idle":
      if (!attention) return null;
      return {
        dotClass: "bg-tyba-green",
        textClass: "text-tyba-green",
        labelKey: "sessionFinished",
        rank: 2,
      };
    case "running":
      return {
        dotClass:
          "bg-tyba-blue [box-shadow:var(--tyba-glow-blue)] motion-safe:animate-pulse",
        textClass: "text-tyba-blue",
        labelKey: "sessionInProgress",
        rank: 1,
      };
    case "exited":
      return null;
  }
};

import type { ForgeStatus, PullRequest } from "./ipc";

export function shouldShowPrIcon(
  status: ForgeStatus | null | undefined,
): boolean {
  return status != null;
}

export function sortPullRequestsByNumberDesc(
  prs: PullRequest[],
): PullRequest[] {
  return [...prs].sort((a, b) => b.number - a.number);
}

export type PrStatusKind = "draft" | "open" | "merged" | "closed";

export function toPrStatus(state: string): PrStatusKind {
  switch (state) {
    case "draft":
    case "open":
    case "merged":
    case "closed":
      return state;
    default:
      return "open";
  }
}

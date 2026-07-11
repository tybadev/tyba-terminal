import type {
  CheckRun,
  ForgeReviewComment,
  ForgeStatus,
  MergePreview,
  PullRequest,
} from "./ipc";

export type DeliveryAction = "open_pr" | "open_mr" | "merge_only";

export function primaryDeliveryAction(
  status: ForgeStatus | null,
): DeliveryAction {
  if (!status) return "merge_only";
  return status.kind === "gitlab" ? "open_mr" : "open_pr";
}

export function forgeCliReady(status: ForgeStatus): boolean {
  return status.installed && status.authenticated;
}

export type MergeBlockedReason = "base_dirty" | "conflicts" | "nothing_to_merge";

export interface MergeGate {
  enabled: boolean;
  reason: MergeBlockedReason | null;
}

export function mergeGate(preview: MergePreview): MergeGate {
  if (preview.base_dirty) return { enabled: false, reason: "base_dirty" };
  if (preview.conflicts.length > 0) {
    return { enabled: false, reason: "conflicts" };
  }
  if (preview.commits === 0) {
    return { enabled: false, reason: "nothing_to_merge" };
  }
  return { enabled: true, reason: null };
}

export type CheckTone = "success" | "failure" | "pending";

export function checkTone(check: CheckRun): CheckTone {
  if (check.status !== "completed") return "pending";
  const conclusion = check.conclusion ?? "";
  if (conclusion === "") return "pending";
  if (["success", "neutral", "skipped"].includes(conclusion)) return "success";
  return "failure";
}

export function overallChecksTone(checks: CheckRun[]): CheckTone | null {
  if (checks.length === 0) return null;
  const tones = checks.map(checkTone);
  if (tones.includes("failure")) return "failure";
  if (tones.includes("pending")) return "pending";
  return "success";
}

export function buildForgeCommentPrompt(
  pr: PullRequest,
  comments: ForgeReviewComment[],
): string {
  const blocks = comments.map((c) => {
    const where = c.path
      ? `${c.path}${c.line !== null ? `:${c.line}` : ""}`
      : "comentário geral";
    return `${where} — @${c.author}\n${c.body.trim()}`;
  });
  return [
    `Review do PR #${pr.number} (${pr.url}) trouxe ${comments.length} comentário(s):`,
    "",
    blocks.join("\n\n"),
    "",
    "Aplique as mudanças pedidas, mantendo o resto como está.",
  ].join("\n");
}

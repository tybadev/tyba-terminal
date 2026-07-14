import type {
  CheckRun,
  ForgeReviewComment,
  ForgeStatus,
  MergePreview,
  PullRequest,
  WorkflowJob,
  WorkflowRun,
} from "./ipc";

export type DeliveryAction = "open_pr" | "open_mr" | "merge_only";

export function primaryDeliveryAction(
  status: ForgeStatus | null,
): DeliveryAction {
  if (!status) return "merge_only";
  return status.kind === "gitlab" ? "open_mr" : "open_pr";
}

export type DeliveryButtonKind = "view_pr" | "view_mr" | "open_pr" | "open_mr";

export function deliveryButtonKind(
  action: DeliveryAction,
  pr: PullRequest | null,
): DeliveryButtonKind {
  const isMr = action === "open_mr";
  if (pr) return isMr ? "view_mr" : "view_pr";
  return isMr ? "open_mr" : "open_pr";
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

/**
 * Um run/job tem a mesma semântica de tom do CheckRun — status + conclusão —
 * então a regra é uma só. Duplicá-la faria o painel de CI e o de PR divergirem
 * na primeira conclusão nova que o GitHub inventasse.
 */
export function runTone(run: {
  status: string;
  conclusion: string | null;
}): CheckTone {
  return checkTone({ name: "", status: run.status, conclusion: run.conclusion });
}

/** Enquanto houver run rodando, o painel se atualiza; parado, ele para. */
export function hasRunningWork(runs: WorkflowRun[]): boolean {
  return runs.some((r) => r.status !== "completed");
}

/**
 * O que o usuário quer ver primeiro é o que está acontecendo AGORA. Depois, o
 * mais recente. A ordem do `gh` já é por data, mas um run antigo ainda rodando
 * importa mais que um novo já concluído.
 */
export function sortRuns(runs: WorkflowRun[]): WorkflowRun[] {
  const running = (r: WorkflowRun) => (r.status === "completed" ? 1 : 0);
  return [...runs].sort(
    (a, b) =>
      running(a) - running(b) || b.created_at.localeCompare(a.created_at),
  );
}

export function jobTone(job: WorkflowJob): CheckTone {
  return runTone(job);
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

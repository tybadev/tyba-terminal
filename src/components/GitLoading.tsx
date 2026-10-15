import { useTranslation } from "react-i18next";

import { LoadingState, type LoadingStateProps } from "./ui/loading";
import type { Session } from "../lib/ipc";
import { shortSha } from "../lib/loading";
import { dirLabel } from "../lib/workspaceCwd";

/**
 * Carregamento de git local (diff, histórico, merge): quando demora, é repo
 * grande ou outro processo segurando o índice.
 */
export function GitLoading(
  props: Omit<LoadingStateProps, "slowCause" | "op"> & { op?: "diff" | "gitLog" | "merge" },
) {
  const { t } = useTranslation();
  return <LoadingState op="diff" slowCause={t("loadGitSlow")} {...props} />;
}

/**
 * A etapa do diff da sessão: com worktree, a base congelada que o core salvou
 * na criação (o `base_ref` é o sha); sem, o repositório que se está lendo.
 */
export function useDiffStep(session: Session, branch?: string | null) {
  const { t } = useTranslation();
  if (branch) return { step: t("loadDiffBranch", { branch }), em: [branch] };
  const base = session.worktree?.base_ref;
  if (base) {
    const sha = shortSha(base);
    return { step: t("loadDiffBase", { sha }), em: [sha] };
  }
  const repo = session.repo_root ? dirLabel(session.repo_root) : "…";
  return { step: t("loadDiffRepo", { repo }), em: [repo] };
}

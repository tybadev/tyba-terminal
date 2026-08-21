import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowDown,
  ArrowUp,
  FolderSimple,
  GitBranch,
  GitDiff,
  TextAlignLeft,
} from "@phosphor-icons/react";

import { Shortcut } from "@/components/ui/kbd";
import type { RepoSnapshot, SessionId } from "../lib/ipc";
import type { ChipId, ToolbarPref } from "../lib/repoSnapshots";
import { BranchPicker } from "./BranchPicker";

interface Props {
  pref: ToolbarPref;
  cwd: string | null;
  /**
   * A sessão de que a barra fala. Sem ela o chip de branch é só texto: trocar
   * de branch é uma operação de repositório, e quem sabe qual repositório é o
   * core, a partir da sessão.
   */
  sessionId: SessionId | null;
  snapshot: RepoSnapshot | undefined;
  hasWorktree: boolean;
  onOpenDiff: () => void;
  showRichInput: boolean;
  richInputCombo: string;
  onOpenRichInput: () => void;
}

function ClockChip() {
  const { i18n } = useTranslation();
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(id);
  }, []);
  const time = new Intl.DateTimeFormat(i18n.language, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(now);
  return <span className="font-mono tabular-nums">{time}</span>;
}

function basename(path: string): string {
  const trimmed = path.endsWith("/") ? path.slice(0, -1) : path;
  const last = trimmed.split("/").pop();
  return last || trimmed;
}

export function Toolbar({
  pref,
  cwd,
  sessionId,
  snapshot,
  hasWorktree,
  onOpenDiff,
  showRichInput,
  richInputCombo,
  onOpenRichInput,
}: Props) {
  const { t } = useTranslation();
  if (!pref.enabled) return null;

  const chip = (id: ChipId) => {
    if (pref.hidden.includes(id)) return null;
    switch (id) {
      case "cwd":
        return cwd ? (
          <span
            key={id}
            title={cwd}
            className="flex min-w-0 shrink items-center gap-1"
            aria-label={t("toolbarCwd")}
          >
            <FolderSimple size={12} className="shrink-0" />
            <span className="truncate">{basename(cwd)}</span>
          </span>
        ) : null;
      case "branch":
        if (!snapshot?.branch) return null;
        // Com sessão, o chip troca de branch; sem ela, continua sendo o texto
        // que sempre foi. O seletor faz o checkout em dois passos — armar e
        // confirmar —, que é a rede para uma ação que mexe no working tree.
        return sessionId ? (
          <BranchPicker
            key={id}
            sessionId={sessionId}
            browsing={null}
            label={snapshot.branch}
            onBrowse={() => {}}
            variant="chip"
            browsable={false}
          />
        ) : (
          <span
            key={id}
            title={snapshot.branch}
            className="flex min-w-0 shrink items-center gap-1"
            aria-label={t("toolbarBranch")}
          >
            <GitBranch size={12} className="shrink-0" />
            <span className="truncate">{snapshot.branch}</span>
          </span>
        );
      case "diffCount": {
        const status = snapshot?.status;
        if (!status?.dirty) return null;
        return (
          <button
            key={id}
            onClick={onOpenDiff}
            title={t("gitStatusIconLabel")}
            aria-label={t("toolbarDiff")}
            className="rounded-[4px] px-1 py-0.5 font-mono transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
          >
            {status.changed} •{" "}
            <span className="text-tyba-green">+{status.insertions}</span>{" "}
            <span className="text-tyba-red">−{status.deletions}</span>
          </button>
        );
      }
      case "reviewDiff":
        return hasWorktree ? (
          <button
            key={id}
            onClick={onOpenDiff}
            title={t("toolbarReviewDiff")}
            aria-label={t("toolbarReviewDiff")}
            className="flex items-center gap-1 rounded-[4px] px-1 py-0.5 text-tyba-text-muted transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
          >
            <GitDiff size={12} />
            <span>diff</span>
          </button>
        ) : null;
      case "aheadBehind": {
        const ahead = snapshot?.ahead ?? 0;
        const behind = snapshot?.behind ?? 0;
        if (snapshot?.ahead == null || (ahead === 0 && behind === 0)) {
          return null;
        }
        return (
          <span
            key={id}
            className="flex items-center gap-0.5 font-mono"
            aria-label={t("toolbarAheadBehind")}
          >
            {ahead > 0 && (
              <>
                <ArrowUp size={11} />
                {ahead}
              </>
            )}
            {behind > 0 && (
              <>
                <ArrowDown size={11} />
                {behind}
              </>
            )}
          </span>
        );
      }
      case "clock":
        return <ClockChip key={id} />;
    }
  };

  const left = pref.left.map(chip).filter(Boolean);
  const right = pref.right.map(chip).filter(Boolean);
  if (!showRichInput && left.length === 0 && right.length === 0) return null;

  return (
    <div className="tyba-divide-t flex h-6 min-w-0 shrink-0 items-center justify-between gap-3 px-3 text-[11px] text-tyba-text-muted">
      <div className="flex min-w-0 items-center gap-3 overflow-hidden">
        {showRichInput && (
          <button
            onClick={onOpenRichInput}
            title={t("richInputHint")}
            className="flex shrink-0 items-center gap-1.5 rounded-[4px] px-1.5 py-0.5 text-tyba-text-muted transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
          >
            <TextAlignLeft size={12} />
            <span>{t("richInputToggle")}</span>
            <Shortcut combo={richInputCombo} />
          </button>
        )}
        {left}
      </div>
      <div className="flex shrink-0 items-center gap-3">{right}</div>
    </div>
  );
}

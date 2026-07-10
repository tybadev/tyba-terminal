import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowDown,
  ArrowUp,
  FolderSimple,
  GitBranch,
  TextAlignLeft,
} from "@phosphor-icons/react";

import { Shortcut } from "@/components/ui/kbd";
import type { RepoSnapshot } from "../lib/ipc";
import type { ChipId, ToolbarPref } from "../lib/repoSnapshots";

interface Props {
  pref: ToolbarPref;
  cwd: string | null;
  snapshot: RepoSnapshot | undefined;
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
  snapshot,
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
            className="flex items-center gap-1"
            aria-label={t("toolbarCwd")}
          >
            <FolderSimple size={12} />
            {basename(cwd)}
          </span>
        ) : null;
      case "branch":
        return snapshot?.branch ? (
          <span
            key={id}
            className="flex items-center gap-1"
            aria-label={t("toolbarBranch")}
          >
            <GitBranch size={12} />
            {snapshot.branch}
          </span>
        ) : null;
      case "diffCount": {
        const status = snapshot?.status;
        if (!status?.dirty) return null;
        return (
          <span key={id} className="font-mono" aria-label={t("toolbarDiff")}>
            {status.changed} •{" "}
            <span className="text-tyba-green">+{status.insertions}</span>{" "}
            <span className="text-tyba-red">−{status.deletions}</span>
          </span>
        );
      }
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
    <div className="flex h-6 shrink-0 items-center justify-between border-t border-tyba-border px-3 text-[11px] text-tyba-text-muted">
      <div className="flex items-center gap-3">
        {showRichInput && (
          <button
            onClick={onOpenRichInput}
            title={t("richInputHint")}
            className="flex items-center gap-1.5 rounded-[4px] px-1.5 py-0.5 text-tyba-text-muted transition-colors hover:bg-white/[.06] hover:text-tyba-text"
          >
            <TextAlignLeft size={12} />
            <span>{t("richInputToggle")}</span>
            <Shortcut combo={richInputCombo} />
          </button>
        )}
        {left}
      </div>
      <div className="flex items-center gap-3">{right}</div>
    </div>
  );
}

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GitBranch, ShieldCheck } from "@phosphor-icons/react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  worktreeSetupScript,
  worktreeSetSetupConsent,
  type SetupScriptInfo,
} from "../lib/ipc";
import { basename } from "@/lib/utils";

interface Props {
  dir: string | null;
  onClose: () => void;
  onCreate: (task: string) => Promise<void>;
}

export function WorktreeCreateDialog({ dir, onClose, onCreate }: Props) {
  const { t } = useTranslation();
  const [task, setTask] = useState("");
  const [script, setScript] = useState<SetupScriptInfo | null>(null);
  const [allowSetup, setAllowSetup] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!dir) return;
    setTask("");
    setError(null);
    setBusy(false);
    setScript(null);
    setAllowSetup(false);
    void worktreeSetupScript(dir)
      .then((info) => {
        setScript(info);
        setAllowSetup(info?.consent === true);
      })
      .catch(() => setScript(null));
  }, [dir]);

  if (!dir) return null;

  const consented = script?.consent === true;

  const create = async () => {
    const title = task.trim();
    if (!title || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (script && allowSetup !== consented) {
        await worktreeSetSetupConsent(dir, script.hash, allowSetup);
      }
      await onCreate(title);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-[520px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[14px]">
            <GitBranch size={16} className="text-tyba-green" />
            {t("worktreeNewSession")}
          </DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {basename(dir)}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-1.5">
          <span className="tyba-label">{t("worktreeTaskTitle")}</span>
          <Input
            autoFocus
            value={task}
            placeholder={t("worktreeTaskPlaceholder")}
            onChange={(e) => setTask(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void create();
            }}
          />
        </div>

        {script && (
          <div className="flex flex-col gap-2 rounded-[6px] border border-tyba-border p-3">
            <div className="flex items-center gap-2 text-[12px] text-tyba-text">
              <ShieldCheck size={14} className="text-tyba-green" />
              {t("worktreeSetupFound")}
            </div>
            <pre className="max-h-40 overflow-auto rounded-[4px] bg-black/30 p-2 text-[11px] leading-relaxed text-tyba-text-muted">
              {script.content}
            </pre>
            <label className="flex items-center justify-between gap-3 text-[12px] text-tyba-text">
              <span className="text-tyba-text-faint">
                {consented ? t("worktreeSetupConsented") : t("worktreeSetupHint")}
              </span>
              <Switch checked={allowSetup} onCheckedChange={setAllowSetup} />
            </label>
          </div>
        )}

        {error && (
          <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
            {t("cancel")}
          </Button>
          <Button size="sm" onClick={() => void create()} disabled={!task.trim() || busy}>
            {busy ? t("worktreeCreating") : t("worktreeCreate")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

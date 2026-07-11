import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GitBranch, Robot, ShieldCheck } from "@phosphor-icons/react";

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
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  agentRepoConfig,
  setAgentConfigConsent,
  worktreeSetupScript,
  worktreeSetSetupConsent,
  type AgentRepoConfig,
  type SetupScriptInfo,
} from "../lib/ipc";
import { needsConsentPrompt, applyConsentDecision } from "../lib/agentSession";
import { AgentConsentDialog } from "./AgentConsentDialog";
import { ClaudeIcon } from "./icons/ClaudeIcon";
import { basename } from "@/lib/utils";

export interface AgentCreateOptions {
  runner: "claude_code";
  prompt: string;
}

interface Props {
  dir: string | null;
  onClose: () => void;
  onCreate: (task: string, agent: AgentCreateOptions | null) => Promise<void>;
}

export function WorktreeCreateDialog({ dir, onClose, onCreate }: Props) {
  const { t } = useTranslation();
  const [task, setTask] = useState("");
  const [script, setScript] = useState<SetupScriptInfo | null>(null);
  const [allowSetup, setAllowSetup] = useState(false);
  const [agentMode, setAgentMode] = useState(false);
  const [agentPrompt, setAgentPrompt] = useState("");
  const [agentConfig, setAgentConfig] = useState<AgentRepoConfig | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!dir) return;
    setTask("");
    setError(null);
    setBusy(false);
    setScript(null);
    setAllowSetup(false);
    setAgentMode(false);
    setAgentPrompt("");
    setAgentConfig(null);
    setConsentOpen(false);
    void worktreeSetupScript(dir)
      .then((info) => {
        setScript(info);
        setAllowSetup(info?.consent === true);
      })
      .catch(() => setScript(null));
    void agentRepoConfig(dir)
      .then(setAgentConfig)
      .catch(() => setAgentConfig(null));
  }, [dir]);

  if (!dir) return null;

  const consented = script?.consent === true;

  const runCreate = async (title: string) => {
    setBusy(true);
    setError(null);
    try {
      if (script && allowSetup !== consented) {
        await worktreeSetSetupConsent(dir, script.hash, allowSetup);
      }
      await onCreate(
        title,
        agentMode ? { runner: "claude_code", prompt: agentPrompt.trim() } : null,
      );
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const create = async () => {
    const title = task.trim();
    if (!title || busy) return;
    if (agentMode && needsConsentPrompt(agentConfig)) {
      setConsentOpen(true);
      return;
    }
    await runCreate(title);
  };

  const decideConsent = async (allow: boolean) => {
    setConsentOpen(false);
    if (agentConfig) {
      const outcome = applyConsentDecision(
        agentConfig,
        allow ? "allow" : "skip",
      );
      setAgentConfig(outcome.config);
      if (outcome.persist) {
        await setAgentConfigConsent(dir, agentConfig.hash, true).catch(
          () => {},
        );
      }
    }
    await runCreate(task.trim());
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

        <label className="flex items-center justify-between gap-3 rounded-[6px] border border-tyba-border p-3 text-[12px] text-tyba-text">
          <span className="flex items-center gap-2">
            <Robot size={15} className="text-tyba-amber" />
            {t("worktreeAgentMode")}
          </span>
          <Switch checked={agentMode} onCheckedChange={setAgentMode} />
        </label>

        {agentMode && (
          <div className="flex flex-col gap-3 rounded-[6px] border border-tyba-border p-3">
            <div className="flex flex-col gap-1.5">
              <span className="tyba-label">{t("worktreeAgentRunner")}</span>
              <div className="flex gap-2">
                <span
                  aria-pressed="true"
                  className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-tyba-green/50 bg-tyba-green/10 px-3 text-sm font-medium text-tyba-text"
                >
                  <ClaudeIcon size={14} style={{ color: "#d97757" }} />
                  Claude Code
                </span>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled
                      >
                        Codex
                      </Button>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{t("worktreeAgentPhase5")}</TooltipContent>
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled
                      >
                        {t("worktreeAgentCustom")}
                      </Button>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{t("worktreeAgentPhase5")}</TooltipContent>
                </Tooltip>
              </div>
            </div>

            <div className="flex flex-col gap-1.5">
              <span className="tyba-label">{t("worktreeAgentPrompt")}</span>
              <textarea
                value={agentPrompt}
                placeholder={t("worktreeAgentPromptPlaceholder")}
                onChange={(e) => setAgentPrompt(e.target.value)}
                rows={3}
                className="max-h-40 min-h-16 resize-none rounded-[4px] border border-tyba-border bg-transparent px-2 py-1.5 font-mono text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-green/50"
              />
            </div>
          </div>
        )}

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

      <AgentConsentDialog
        config={consentOpen ? agentConfig : null}
        onAllow={() => void decideConsent(true)}
        onSkip={() => void decideConsent(false)}
      />
    </Dialog>
  );
}

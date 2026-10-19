import { useTranslation } from "react-i18next";
import { ShieldWarning } from "@phosphor-icons/react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import type { AgentRepoConfig } from "../lib/ipc";

interface Props {
  config: AgentRepoConfig | null;
  onAllow: () => void;
  onSkip: () => void;
}

/** O `.tyba/config.toml` do repo só vale com consentimento: ele escolhe o
 * agente padrão e põe snippets — comandos — na paleta do dono. */
export function AgentConsentDialog({ config, onAllow, onSkip }: Props) {
  const { t } = useTranslation();
  if (!config) return null;

  return (
    <Dialog open onOpenChange={(open) => !open && onSkip()}>
      <DialogContent className="max-w-[480px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[14px]">
            <ShieldWarning size={16} className="text-tyba-amber" />
            {t("agentConsentTitle")}
          </DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {t("agentConsentBody")}
          </DialogDescription>
        </DialogHeader>

        <ul className="flex flex-col gap-1 rounded-[6px] border border-tyba-border p-3 font-mono text-[12px] text-tyba-text">
          {config.default_agent && (
            <li className="truncate">
              {t("agentConsentDefaultAgent", { agent: config.default_agent })}
            </li>
          )}
          {config.snippets.map((snippet) => (
            <li key={snippet} className="truncate">
              {snippet}
            </li>
          ))}
        </ul>

        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onSkip}>
            {t("agentConsentSkip")}
          </Button>
          <Button size="sm" onClick={onAllow}>
            {t("agentConsentAllow")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

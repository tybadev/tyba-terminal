import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ShieldCheck } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  claudePluginStatus,
  installClaudePlugin,
  uninstallClaudePlugin,
  type ClaudePluginStatus,
} from "../lib/ipc";
import { translateError } from "../lib/errors";
import { toastError } from "../lib/toast";

/**
 * M6-B: Configurações → Integrações → "Integração com Claude Code". O plugin
 * mexe na config global do Claude, então só instala ou desinstala por clique
 * aqui. Quem decide se está instalado ou bloqueado é o core.
 */
export function ClaudePluginSetting() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<ClaudePluginStatus | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    claudePluginStatus()
      .then((s) => {
        if (alive) setStatus(s);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const change = (action: () => Promise<ClaudePluginStatus>) => {
    setBusy(true);
    action()
      .then(setStatus)
      .catch((e) => toastError(t("claudePluginFailed"), translateError(e, t)))
      .finally(() => setBusy(false));
  };

  const blocked =
    status?.blocked === "disable_all_hooks"
      ? t("claudePluginBlockedAll")
      : status?.blocked === "managed_hooks_only"
        ? t("claudePluginBlockedManaged")
        : null;

  return (
    <div className="mt-2 flex items-start gap-3 py-3">
      <span className="mt-0.5 shrink-0 text-tyba-text-muted">
        <ShieldCheck size={18} />
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] text-tyba-text">{t("claudePlugin")}</p>
        <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
          {t("claudePluginHint")}
        </p>
        {status && !status.available && (
          <p className="pt-1 text-[11px] text-tyba-text-faint">
            {t("claudePluginUnavailable")}
          </p>
        )}
        {blocked && (
          <p className="pt-1 text-[11px] text-tyba-red">{blocked}</p>
        )}
        {status?.installed && (
          <p className="pt-1 text-[11px] text-tyba-text-muted">
            {t("claudePluginInstalled")}
          </p>
        )}
      </div>
      {status?.installed ? (
        <Button
          variant="ghost"
          size="sm"
          disabled={busy}
          onClick={() => change(uninstallClaudePlugin)}
        >
          {t("claudePluginUninstall")}
        </Button>
      ) : (
        <Button
          variant="outline"
          size="sm"
          disabled={busy || !status?.available}
          onClick={() => change(installClaudePlugin)}
        >
          {t("claudePluginInstall")}
        </Button>
      )}
    </div>
  );
}

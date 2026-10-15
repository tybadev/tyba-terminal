import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  claudePluginStatus,
  installClaudePlugin,
  uninstallClaudePlugin,
  type ClaudePluginStatus,
} from "../lib/ipc";
import { translateError } from "../lib/errors";
import { toastError } from "../lib/toast";
import { SettingLine, SettingNote, SettingStatus } from "./settings/rows";

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
    <>
      <SettingLine label={t("claudePlugin")} hint={t("claudePluginHint")}>
        {status && !status.available ? (
          <SettingStatus tone="faint">{t("claudePluginUnavailableShort")}</SettingStatus>
        ) : status?.installed ? (
          <SettingStatus tone="green">{t("claudePluginInstalled")}</SettingStatus>
        ) : (
          <SettingStatus tone="faint">{t("claudePluginNotInstalled")}</SettingStatus>
        )}
        {status?.installed ? (
          <Button variant="ghost" size="sm" disabled={busy} onClick={() => change(uninstallClaudePlugin)}>
            {t("claudePluginUninstall")}
          </Button>
        ) : (
          <Button
            variant="outline"
            size="sm"
            disabled={busy || !status?.available}
            onClick={() => change(installClaudePlugin)}
          >
            {t("claudePluginInstallShort")}
          </Button>
        )}
      </SettingLine>
      {status && !status.available && <SettingNote>{t("claudePluginUnavailable")}</SettingNote>}
      {blocked && <SettingNote tone="amber">{blocked}</SettingNote>}
    </>
  );
}

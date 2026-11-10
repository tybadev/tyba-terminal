import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Info, Plus, X } from "@phosphor-icons/react";

import { Switch } from "@/components/ui/switch";
import {
  sshTypedAllow,
  sshTypedDeny,
  sshTypedPrefs,
  sshTypedSet,
  type TypedSshMode,
  type TypedSshPrefs,
} from "../../lib/ipc";
import { dayMonth, targetLabel } from "../../lib/typedSsh";
import { toastError } from "../../lib/toast";
import { SettingGroup, SettingLine } from "./rows";

const MODES: { id: TypedSshMode; label: string }[] = [
  { id: "ask", label: "sshTypedAsk" },
  { id: "open", label: "sshTypedOpen" },
  { id: "never", label: "sshTypedNever" },
];

/**
 * Configurações › SSH (quadro 2 de SshVariaveisFila): o `ssh` digitado. O
 * estado mora no core (`ssh::typed_prefs`), que é quem decide se o aviso sai;
 * aqui só se lê e se pede mudança.
 */
export function SshSection() {
  const { t } = useTranslation();
  const [prefs, setPrefs] = useState<TypedSshPrefs | null>(null);
  const [draft, setDraft] = useState("");

  useEffect(() => {
    void sshTypedPrefs()
      .then(setPrefs)
      .catch(() => {});
  }, []);

  const apply = (call: Promise<TypedSshPrefs>, onOk?: () => void) =>
    void call
      .then((next) => {
        setPrefs(next);
        onOk?.();
      })
      .catch((e) => toastError(t("sshTypedSaveFailed"), e));

  const add = () => {
    if (!draft.trim()) return;
    apply(sshTypedDeny(draft), () => setDraft(""));
  };

  if (!prefs) return null;
  return (
    <>
      <SettingGroup title={t("sshIntegration")} />
      <SettingLine label={t("sshTypedDetect")} hint={t("sshTypedDetectHint")}>
        <Switch
          checked={prefs.detect}
          onCheckedChange={(detect) => apply(sshTypedSet({ detect }))}
          aria-label={t("sshTypedDetect")}
        />
      </SettingLine>
      <SettingLine label={t("sshTypedWhen")} hint={t("sshTypedWhenHint")}>
        <div
          role="radiogroup"
          aria-label={t("sshTypedWhen")}
          className="tyba-segment"
          style={{ background: "color-mix(in srgb, var(--tyba-violet) 9%, transparent)" }}
        >
          {MODES.map((m) => (
            <button
              key={m.id}
              role="radio"
              aria-checked={prefs.mode === m.id}
              onClick={() => apply(sshTypedSet({ mode: m.id }))}
              className={`flex h-[26px] items-center rounded-[5px] px-3 text-[12px] ${
                prefs.mode === m.id
                  ? "tyba-pill-on text-tyba-text"
                  : "text-tyba-text-faint hover:text-tyba-text"
              }`}
            >
              {t(m.label)}
            </button>
          ))}
        </div>
      </SettingLine>
      <div className="flex items-start gap-2 border-b border-tyba-border-soft px-[18px] py-2.5 text-[12px] leading-[18px] text-tyba-text-faint">
        <Info size={14} className="mt-0.5 shrink-0" />
        <span>{t("sshTypedRcNote")}</span>
      </div>
      <SettingGroup
        title={t("sshDenied")}
        note={t("sshDeniedNote", { count: prefs.denied.length })}
      />
      {prefs.denied.map((d) => {
        const label = targetLabel(d);
        return (
          <div
            key={label}
            data-setting-row
            className="grid grid-cols-[1fr_auto_auto] items-center gap-4 border-b border-tyba-border-soft px-[18px] py-[7px] font-mono text-[12.5px]"
          >
            <span className="min-w-0 truncate text-tyba-text">{label}</span>
            <span className="text-[11px] text-tyba-text-faint">
              {t("sshDeniedSince", { date: dayMonth(d.since_ms) })}
            </span>
            <button
              onClick={() => apply(sshTypedAllow(label))}
              aria-label={t("sshDeniedRemove", { host: label })}
              className="flex size-6 items-center justify-center rounded-[5px] text-tyba-text-faint hover:bg-tyba-text/[.04] hover:text-tyba-text"
            >
              <X size={13} />
            </button>
          </div>
        );
      })}
      <div className="grid grid-cols-[1fr_auto] items-center gap-4 border-b border-tyba-border-soft px-[18px] py-[7px]">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") add();
          }}
          placeholder={t("sshDeniedPlaceholder")}
          aria-label={t("sshDeniedPlaceholder")}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          className="h-7 min-w-0 rounded-[5px] bg-tyba-text/[.02] px-[9px] font-mono text-[12px] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border)] outline-none placeholder:text-tyba-text-ghost focus:shadow-[inset_0_0_0_1px_var(--tyba-border-strong)]"
        />
        <button
          onClick={add}
          className="flex h-6 items-center gap-1.5 rounded-[6px] px-[9px] text-[11.5px] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border-field)] hover:bg-tyba-text/[.04]"
        >
          <Plus size={12} />
          {t("sshDeniedAdd")}
        </button>
      </div>
    </>
  );
}

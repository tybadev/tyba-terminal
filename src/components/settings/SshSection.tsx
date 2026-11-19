import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Info, PenNib, Plus, X } from "@phosphor-icons/react";

import { Switch } from "@/components/ui/switch";
import { HostFormDialog, type HostDialogState } from "../HostFormDialog";
import {
  listHostGroups,
  listHosts,
  onHostsChanged,
  type Host,
  type HostGroup,
  sshTypedAllow,
  sshTypedDeny,
  sshTypedPrefs,
  sshTypedSet,
  type TypedSshMode,
  type TypedSshPrefs,
} from "../../lib/ipc";
import { dayMonth, targetLabel } from "../../lib/typedSsh";
import { toastError } from "../../lib/toast";
import { SETTING_FIELD, SettingButton, SettingGroup, SettingLine, SettingNote, SettingSegment } from "./rows";

const MODES: { id: TypedSshMode; label: string }[] = [
  { id: "ask", label: "sshTypedAsk" },
  { id: "open", label: "sshTypedOpen" },
  { id: "never", label: "sshTypedNever" },
];

/**
 * Configurações › Integrações › Hosts SSH (quadro "Configurações (aprovada)";
 * conteúdo do quadro 2 de SshVariaveisFila): o `ssh` digitado. O
 * estado mora no core (`ssh::typed_prefs`), que é quem decide se o aviso sai;
 * aqui só se lê e se pede mudança.
 */
export function SshSection() {
  const { t } = useTranslation();
  const [prefs, setPrefs] = useState<TypedSshPrefs | null>(null);
  const [draft, setDraft] = useState("");
  // Os Hosts cadastrados (quadro: lista com Editar e "Adicionar host"). O
  // core é dono da lista e avisa a cada gravação (`ssh://hosts-changed`).
  const [hosts, setHosts] = useState<Host[]>([]);
  const [groups, setGroups] = useState<HostGroup[]>([]);
  const [editor, setEditor] = useState<HostDialogState>(null);

  useEffect(() => {
    void sshTypedPrefs()
      .then(setPrefs)
      .catch(() => {});
  }, []);

  useEffect(() => {
    const load = () => {
      void listHosts()
        .then(setHosts)
        .catch(() => {});
      void listHostGroups()
        .then(setGroups)
        .catch(() => {});
    };
    load();
    const unlisten = onHostsChanged(load);
    return () => {
      void unlisten.then((off) => off());
    };
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
    <SettingGroup title={t("cfgGroupSsh")}>
      {hosts.map((host) => (
        <SettingLine
          key={host.id}
          label={host.alias}
          hint={
            <span className="font-mono">
              {host.username ? `${host.username}@` : ""}
              {host.hostname}
              {host.port ? `:${host.port}` : ""}
            </span>
          }
        >
          <SettingButton
            tone="ghost"
            icon={<PenNib size={16} />}
            onClick={() => setEditor({ mode: "edit", host })}
            aria-label={t("cfgSshHostEditAria", { host: host.alias })}
          >
            {t("cfgSshHostEdit")}
          </SettingButton>
        </SettingLine>
      ))}
      <SettingLine>
        <SettingButton icon={<Plus size={16} />} onClick={() => setEditor({ mode: "create" })}>
          {t("cfgSshHostAdd")}
        </SettingButton>
      </SettingLine>
      <HostFormDialog
        state={editor}
        groups={groups}
        onClose={() => setEditor(null)}
        onSaved={() => setEditor(null)}
      />
      <SettingLine label={t("sshTypedDetect")} hint={t("sshTypedDetectHint")}>
        <Switch
          checked={prefs.detect}
          onCheckedChange={(detect) => apply(sshTypedSet({ detect }))}
          aria-label={t("sshTypedDetect")}
        />
      </SettingLine>
      <SettingLine label={t("sshTypedWhen")} hint={t("sshTypedWhenHint")}>
        <SettingSegment
          label={t("sshTypedWhen")}
          options={MODES.map((m) => ({ value: m.id, label: t(m.label) }))}
          value={prefs.mode}
          onChange={(mode) => apply(sshTypedSet({ mode }))}
        />
      </SettingLine>
      <SettingNote>
        <span className="flex items-start gap-2">
          <Info size={14} className="mt-0.5 shrink-0" />
          {t("sshTypedRcNote")}
        </span>
      </SettingNote>
      {prefs.denied.map((d) => {
        const label = targetLabel(d);
        return (
          <SettingLine
            key={label}
            label={<span className="font-mono text-[13px]">{label}</span>}
            hint={t("cfgSshDeniedSince", { date: dayMonth(d.since_ms) })}
          >
            <SettingButton
              tone="ghost"
              icon={<X size={14} />}
              onClick={() => apply(sshTypedAllow(label))}
              aria-label={t("sshDeniedRemove", { host: label })}
            >
              {t("cfgRemove")}
            </SettingButton>
          </SettingLine>
        );
      })}
      <SettingLine hint={t("cfgSshDenyHint")}>
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
          className={`${SETTING_FIELD} px-3 font-mono text-[12.5px] text-tyba-text outline-none placeholder:text-tyba-text-ghost focus:shadow-[0_0_0_1px_var(--tyba-border-strong)]`}
        />
        <SettingButton icon={<Plus size={14} />} onClick={add}>
          {t("cfgSshDenyAdd")}
        </SettingButton>
      </SettingLine>
    </SettingGroup>
  );
}

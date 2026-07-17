import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  CaretRight,
  FolderPlus,
  HardDrives,
  PencilSimple,
  Plug,
  Plugs,
  Plus,
  Prohibit,
  Trash,
  Warning,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select } from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Toast,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
} from "@/components/ui/toast";
import { translateError } from "@/lib/errors";
import {
  listHosts,
  listHostGroups,
  createHost,
  updateHost,
  deleteHost,
  createHostGroup,
  updateHostGroup,
  deleteHostGroup,
  type Host,
  type HostGroup,
  type HostInput,
  type HostGroupInput,
  type Tunnel,
  type TunnelKind,
} from "@/lib/ipc";
import { BLANK_TUNNEL, addedRiskyTunnels } from "@/lib/tunnels";
import { RiskyTunnelConfirm } from "@/components/RiskyTunnelConfirm";

const UNGROUPED_KEY = "__ungrouped__";
const NO_GROUP_VALUE = "__none__";
const ACTION_ERROR_MS = 8000;

const CONNECTION_COLORS = [
  "green",
  "amber",
  "magenta",
  "violet",
  "blue",
  "cyan",
  "red",
];

interface Props {
  onConnect: (host: Host) => void;
  /** Abre o grupo como panes de um workspace só (base do broadcast). */
  onConnectGroup?: (group: HostGroup | null, hosts: Host[]) => void;
}

interface HostFormValues {
  alias: string;
  hostname: string;
  port: string;
  username: string;
  identity_file: string;
  proxy_jump: string;
  group_id: string;
  color: string | null;
  notes: string;
  tunnels: Tunnel[];
}

function validPort(p: number | null): boolean {
  return p !== null && Number.isInteger(p) && p >= 1 && p <= 65535;
}

function normalizeTunnelDraft(d: Tunnel): Tunnel | null {
  if (!validPort(d.listen_port)) return null;
  if (d.kind === "dynamic") return { ...d, target_host: null, target_port: null };
  const target = d.target_host?.trim();
  if (!target || !validPort(d.target_port)) return null;
  return { ...d, target_host: target };
}

function tunnelRowHasInput(d: Tunnel): boolean {
  return d.listen_port > 0 || (d.target_port ?? 0) > 0;
}

interface HostGroupFormValues {
  name: string;
  color: string | null;
  notes: string;
}

type HostDialogState =
  | { mode: "create" }
  | { mode: "edit"; host: Host }
  | null;

type GroupDialogState =
  | { mode: "create" }
  | { mode: "edit"; group: HostGroup }
  | null;

function emptyHostForm(): HostFormValues {
  return {
    alias: "",
    hostname: "",
    port: "",
    username: "",
    identity_file: "",
    proxy_jump: "",
    group_id: NO_GROUP_VALUE,
    color: null,
    notes: "",
    tunnels: [],
  };
}

function hostToForm(host: Host): HostFormValues {
  return {
    alias: host.alias,
    hostname: host.hostname,
    port: host.port === null ? "" : String(host.port),
    username: host.username ?? "",
    identity_file: host.identity_file ?? "",
    proxy_jump: host.proxy_jump ?? "",
    group_id: host.group_id ?? NO_GROUP_VALUE,
    color: host.color,
    notes: host.notes ?? "",
    tunnels: host.tunnels,
  };
}

function emptyGroupForm(): HostGroupFormValues {
  return { name: "", color: null, notes: "" };
}

function groupToForm(group: HostGroup): HostGroupFormValues {
  return { name: group.name, color: group.color, notes: group.notes ?? "" };
}

function formatLastConnected(iso: string, locale: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const diffMs = date.getTime() - Date.now();
  const diffMinutes = Math.round(diffMs / 60000);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  if (Math.abs(diffMinutes) < 60) return rtf.format(diffMinutes, "minute");
  const diffHours = Math.round(diffMinutes / 60);
  if (Math.abs(diffHours) < 24) return rtf.format(diffHours, "hour");
  const diffDays = Math.round(diffHours / 24);
  if (Math.abs(diffDays) < 30) return rtf.format(diffDays, "day");
  const diffMonths = Math.round(diffDays / 30);
  if (Math.abs(diffMonths) < 12) return rtf.format(diffMonths, "month");
  const diffYears = Math.round(diffMonths / 12);
  return rtf.format(diffYears, "year");
}

function PanelAction({
  label,
  onClick,
  destructive,
  children,
}: {
  label: string;
  onClick: () => void;
  destructive?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          onClick={onClick}
          className={`rounded-[3px] p-1 transition-colors ${
            destructive
              ? "text-tyba-text-faint hover:text-tyba-red"
              : "text-tyba-text-faint hover:text-tyba-text"
          }`}
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

function ColorPicker({
  value,
  onChange,
  noColorLabel,
}: {
  value: string | null;
  onChange: (value: string | null) => void;
  noColorLabel: string;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <button
        type="button"
        aria-label={noColorLabel}
        onClick={() => onChange(null)}
        className={`flex size-5 shrink-0 items-center justify-center rounded-full border text-tyba-text-faint ${
          value === null ? "border-tyba-text-muted" : "border-tyba-border-strong"
        }`}
      >
        <Prohibit size={11} />
      </button>
      {CONNECTION_COLORS.map((c) => (
        <button
          key={c}
          type="button"
          aria-label={c}
          onClick={() => onChange(c)}
          className={`size-5 shrink-0 rounded-full border ${
            value === c ? "border-tyba-text" : "border-transparent"
          }`}
          style={{ background: `var(--tyba-${c})` }}
        />
      ))}
    </div>
  );
}

function FormField({
  label,
  htmlFor,
  hint,
  children,
}: {
  label: string;
  htmlFor: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={htmlFor} className="tyba-label">
        {label}
      </label>
      {children}
      {hint && <span className="text-[11px] text-tyba-text-faint">{hint}</span>}
    </div>
  );
}

function HostDialog({
  state,
  groups,
  onClose,
  onSaved,
}: {
  state: HostDialogState;
  groups: HostGroup[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [values, setValues] = useState<HostFormValues>(emptyHostForm());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tunnelError, setTunnelError] = useState<string | null>(null);
  const [confirmRisky, setConfirmRisky] = useState<Tunnel[] | null>(null);

  useEffect(() => {
    if (state === null) return;
    setValues(state.mode === "edit" ? hostToForm(state.host) : emptyHostForm());
    setError(null);
    setBusy(false);
    setTunnelError(null);
    setConfirmRisky(null);
  }, [state]);

  if (state === null) return null;

  const groupOptions = [
    { value: NO_GROUP_VALUE, label: t("connectionsNoGroup") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ];

  const addTunnelRow = () => {
    setTunnelError(null);
    setConfirmRisky(null);
    setValues((v) => ({ ...v, tunnels: [...v.tunnels, BLANK_TUNNEL] }));
  };

  const patchTunnel = (index: number, patch: Partial<Tunnel>) => {
    setTunnelError(null);
    setConfirmRisky(null);
    setValues((v) => ({
      ...v,
      tunnels: v.tunnels.map((tn, i) => (i === index ? { ...tn, ...patch } : tn)),
    }));
  };

  const removeTunnel = (index: number) => {
    setTunnelError(null);
    setConfirmRisky(null);
    setValues((v) => ({
      ...v,
      tunnels: v.tunnels.filter((_, i) => i !== index),
    }));
  };

  const save = async (confirmed: boolean) => {
    const alias = values.alias.trim();
    const hostname = values.hostname.trim();
    if (!alias) {
      setError(t("hostFieldAliasRequired"));
      return;
    }
    if (!hostname) {
      setError(t("hostFieldHostnameRequired"));
      return;
    }
    let port: number | null = null;
    const portTrimmed = values.port.trim();
    if (portTrimmed) {
      port = Number(portTrimmed);
      if (!validPort(port)) {
        setError(t("hostFieldPortInvalid"));
        return;
      }
    }
    const tunnels: Tunnel[] = [];
    for (const row of values.tunnels) {
      if (!tunnelRowHasInput(row)) continue;
      const norm = normalizeTunnelDraft(row);
      if (norm === null) {
        setTunnelError(t("hostTunnelInvalid"));
        return;
      }
      tunnels.push(norm);
    }
    setTunnelError(null);
    setValues((v) => ({ ...v, tunnels }));
    const groupId = values.group_id === NO_GROUP_VALUE ? null : values.group_id;
    const username = values.username.trim() || null;
    const identityFile = values.identity_file.trim() || null;
    const proxyJump = values.proxy_jump.trim() || null;
    const notes = values.notes.trim() || null;

    setBusy(true);
    setError(null);
    try {
      if (state.mode === "create") {
        const input: HostInput = {
          alias,
          hostname,
          port,
          username,
          identity_file: identityFile,
          proxy_jump: proxyJump,
          group_id: groupId,
          color: values.color,
          notes,
          tunnels,
        };
        await createHost(input, confirmed);
      } else {
        const updated: Host = {
          ...state.host,
          alias,
          hostname,
          port,
          username,
          identity_file: identityFile,
          proxy_jump: proxyJump,
          group_id: groupId,
          color: values.color,
          notes,
          tunnels,
        };
        await updateHost(updated, confirmed);
      }
      onSaved();
    } catch (e) {
      const err = e as { code?: string; params?: Record<string, string> };
      if (err.code === "ssh.tunnel_needs_confirmation") {
        const prev = state.mode === "edit" ? state.host.tunnels : [];
        const risky = addedRiskyTunnels(prev, tunnels);
        setConfirmRisky(
          risky.length > 0
            ? risky
            : [
                {
                  kind: err.params?.kind === "-D" ? "dynamic" : "remote",
                  listen_port: Number(err.params?.port ?? 0),
                  listen_host: null,
                  target_host: null,
                  target_port: null,
                },
              ],
        );
        setBusy(false);
        return;
      }
      setError(translateError(e, t));
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-[520px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[14px]">
            <Plug size={16} className="text-tyba-cyan" />
            {state.mode === "create"
              ? t("hostDialogTitleCreate")
              : t("hostDialogTitleEdit")}
          </DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {t("hostDialogDescription")}
          </DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-2 gap-3">
          <FormField label={t("hostFieldAlias")} htmlFor="host-alias">
            <Input
              id="host-alias"
              autoFocus
              value={values.alias}
              placeholder={t("hostFieldAliasPlaceholder")}
              onChange={(e) => setValues((v) => ({ ...v, alias: e.target.value }))}
            />
          </FormField>
          <FormField label={t("hostFieldHostname")} htmlFor="host-hostname">
            <Input
              id="host-hostname"
              value={values.hostname}
              placeholder={t("hostFieldHostnamePlaceholder")}
              onChange={(e) =>
                setValues((v) => ({ ...v, hostname: e.target.value }))
              }
            />
          </FormField>
          <FormField label={t("hostFieldPort")} htmlFor="host-port">
            <Input
              id="host-port"
              type="number"
              min={1}
              max={65535}
              value={values.port}
              placeholder="22"
              onChange={(e) => setValues((v) => ({ ...v, port: e.target.value }))}
            />
          </FormField>
          <FormField label={t("hostFieldUsername")} htmlFor="host-username">
            <Input
              id="host-username"
              value={values.username}
              placeholder={t("hostFieldUsernamePlaceholder")}
              onChange={(e) =>
                setValues((v) => ({ ...v, username: e.target.value }))
              }
            />
          </FormField>
        </div>

        <FormField
          label={t("hostFieldIdentityFile")}
          htmlFor="host-identity-file"
          hint={t("hostFieldIdentityFileHint")}
        >
          <Input
            id="host-identity-file"
            value={values.identity_file}
            placeholder={t("hostFieldIdentityFilePlaceholder")}
            onChange={(e) =>
              setValues((v) => ({ ...v, identity_file: e.target.value }))
            }
          />
        </FormField>

        <FormField label={t("hostFieldProxyJump")} htmlFor="host-proxy-jump">
          <Input
            id="host-proxy-jump"
            value={values.proxy_jump}
            placeholder={t("hostFieldProxyJumpPlaceholder")}
            onChange={(e) =>
              setValues((v) => ({ ...v, proxy_jump: e.target.value }))
            }
          />
        </FormField>

        <FormField
          label={t("hostFieldTunnels")}
          htmlFor="host-tunnels"
          hint={t("hostFieldTunnelsHint")}
        >
          <div id="host-tunnels" className="flex flex-col gap-1.5">
            {values.tunnels.map((tn, i) => (
              <div key={i} className="flex items-center gap-1.5">
                <select
                  aria-label={t("tunnelsKind")}
                  value={tn.kind}
                  onChange={(e) => {
                    const kind = e.target.value as TunnelKind;
                    patchTunnel(i, {
                      kind,
                      target_host: kind === "dynamic" ? null : (tn.target_host ?? "localhost"),
                      target_port: kind === "dynamic" ? null : tn.target_port,
                    });
                  }}
                  className="h-7 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 text-[11px] text-tyba-text"
                >
                  <option value="local">-L</option>
                  <option value="remote">-R</option>
                  <option value="dynamic">-D</option>
                </select>
                <input
                  aria-label={t("tunnelsListenPort")}
                  type="number"
                  min={1}
                  max={65535}
                  placeholder={t("tunnelsListenPort")}
                  value={tn.listen_port || ""}
                  onChange={(e) =>
                    patchTunnel(i, { listen_port: Number(e.target.value) })
                  }
                  className="h-7 w-20 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 font-mono text-[11px] text-tyba-text"
                />
                {tn.kind !== "dynamic" && (
                  <>
                    <input
                      aria-label={t("tunnelsTargetHost")}
                      placeholder="localhost"
                      value={tn.target_host ?? ""}
                      onChange={(e) =>
                        patchTunnel(i, { target_host: e.target.value })
                      }
                      className="h-7 min-w-0 flex-1 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 font-mono text-[11px] text-tyba-text"
                    />
                    <input
                      aria-label={t("tunnelsTargetPort")}
                      type="number"
                      min={1}
                      max={65535}
                      placeholder={t("tunnelsTargetPort")}
                      value={tn.target_port || ""}
                      onChange={(e) =>
                        patchTunnel(i, { target_port: Number(e.target.value) })
                      }
                      className="h-7 w-20 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 font-mono text-[11px] text-tyba-text"
                    />
                  </>
                )}
                <button
                  type="button"
                  aria-label={t("hostTunnelRemove")}
                  onClick={() => removeTunnel(i)}
                  className="shrink-0 text-tyba-text-faint hover:text-tyba-red"
                >
                  <Trash size={12} />
                </button>
              </div>
            ))}
            {tunnelError && (
              <p className="text-[11px] text-tyba-red">{tunnelError}</p>
            )}
            <button
              type="button"
              onClick={addTunnelRow}
              className="flex items-center gap-1.5 px-1 text-[11px] text-tyba-text-faint hover:text-tyba-text"
            >
              <Plus size={11} />
              {t("tunnelsNew")}
            </button>
          </div>
        </FormField>

        <div className="grid grid-cols-2 gap-3">
          <FormField label={t("hostFieldGroup")} htmlFor="host-group">
            <Select
              value={values.group_id}
              options={groupOptions}
              onChange={(value) => setValues((v) => ({ ...v, group_id: value }))}
            />
          </FormField>
          <FormField label={t("hostFieldColor")} htmlFor="host-color">
            <ColorPicker
              value={values.color}
              onChange={(color) => setValues((v) => ({ ...v, color }))}
              noColorLabel={t("noColor")}
            />
          </FormField>
        </div>

        <FormField label={t("hostFieldNotes")} htmlFor="host-notes">
          <Textarea
            id="host-notes"
            rows={2}
            value={values.notes}
            placeholder={t("hostFieldNotesPlaceholder")}
            onChange={(e) => setValues((v) => ({ ...v, notes: e.target.value }))}
          />
        </FormField>

        {error && (
          <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
            {error}
          </div>
        )}

        {confirmRisky ? (
          <RiskyTunnelConfirm
            tunnels={confirmRisky}
            host={values.alias.trim() || values.hostname.trim()}
            confirmLabel={
              busy ? t("connectionsSaving") : t("hostTunnelConfirmSave")
            }
            busy={busy}
            onConfirm={() => void save(true)}
            onCancel={() => setConfirmRisky(null)}
          />
        ) : (
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
              {t("cancel")}
            </Button>
            <Button size="sm" onClick={() => void save(false)} disabled={busy}>
              {busy ? t("connectionsSaving") : t("connectionsSave")}
            </Button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function GroupDialog({
  state,
  onClose,
  onSaved,
}: {
  state: GroupDialogState;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [values, setValues] = useState<HostGroupFormValues>(emptyGroupForm());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (state === null) return;
    setValues(state.mode === "edit" ? groupToForm(state.group) : emptyGroupForm());
    setError(null);
    setBusy(false);
  }, [state]);

  if (state === null) return null;

  const save = async () => {
    const name = values.name.trim();
    if (!name) {
      setError(t("groupFieldNameRequired"));
      return;
    }
    const notes = values.notes.trim() || null;

    setBusy(true);
    setError(null);
    try {
      if (state.mode === "create") {
        const input: HostGroupInput = { name, color: values.color, notes };
        await createHostGroup(input);
      } else {
        const updated: HostGroup = {
          ...state.group,
          name,
          color: values.color,
          notes,
        };
        await updateHostGroup(updated);
      }
      onSaved();
    } catch (e) {
      setError(translateError(e, t));
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-[420px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[14px]">
            <FolderPlus size={16} className="text-tyba-cyan" />
            {state.mode === "create"
              ? t("groupDialogTitleCreate")
              : t("groupDialogTitleEdit")}
          </DialogTitle>
        </DialogHeader>

        <FormField label={t("groupFieldName")} htmlFor="group-name">
          <Input
            id="group-name"
            autoFocus
            value={values.name}
            placeholder={t("groupFieldNamePlaceholder")}
            onChange={(e) => setValues((v) => ({ ...v, name: e.target.value }))}
          />
        </FormField>

        <FormField label={t("groupFieldColor")} htmlFor="group-color">
          <ColorPicker
            value={values.color}
            onChange={(color) => setValues((v) => ({ ...v, color }))}
            noColorLabel={t("noColor")}
          />
        </FormField>

        <FormField label={t("groupFieldNotes")} htmlFor="group-notes">
          <Textarea
            id="group-notes"
            rows={2}
            value={values.notes}
            placeholder={t("groupFieldNotesPlaceholder")}
            onChange={(e) => setValues((v) => ({ ...v, notes: e.target.value }))}
          />
        </FormField>

        {error && (
          <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
            {t("cancel")}
          </Button>
          <Button size="sm" onClick={() => void save()} disabled={busy}>
            {busy ? t("connectionsSaving") : t("connectionsSave")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function ConnectionsView({ onConnect, onConnectGroup }: Props) {
  const { t, i18n } = useTranslation();
  const [hosts, setHosts] = useState<Host[] | null>(null);
  const [groups, setGroups] = useState<HostGroup[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [confirmingDeleteHost, setConfirmingDeleteHost] = useState<
    string | null
  >(null);
  const [confirmingDeleteGroup, setConfirmingDeleteGroup] = useState<
    string | null
  >(null);
  const [hostDialog, setHostDialog] = useState<HostDialogState>(null);
  const [groupDialog, setGroupDialog] = useState<GroupDialogState>(null);

  const load = useCallback(async () => {
    try {
      const [hostList, groupList] = await Promise.all([
        listHosts(),
        listHostGroups(),
      ]);
      setHosts(hostList);
      setGroups(groupList);
      setLoadError(null);
    } catch (e) {
      setLoadError(translateError(e, t));
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (actionError === null) return;
    const timer = window.setTimeout(
      () => setActionError(null),
      ACTION_ERROR_MS,
    );
    return () => window.clearTimeout(timer);
  }, [actionError]);

  const failAction = useCallback(
    (e: unknown) => setActionError(translateError(e, t)),
    [t],
  );

  const sections = useMemo(() => {
    if (!hosts || !groups) return [];
    const byGroup = new Map<string, Host[]>();
    for (const h of hosts) {
      const key = h.group_id ?? UNGROUPED_KEY;
      byGroup.set(key, [...(byGroup.get(key) ?? []), h]);
    }
    const sortHosts = (list: Host[]) =>
      [...list].sort((a, b) => a.position - b.position);
    const groupSections = [...groups]
      .sort((a, b) => a.position - b.position)
      .map((group) => ({
        key: group.id,
        group,
        items: sortHosts(byGroup.get(group.id) ?? []),
      }));
    const ungrouped = sortHosts(byGroup.get(UNGROUPED_KEY) ?? []);
    return ungrouped.length > 0
      ? [...groupSections, { key: UNGROUPED_KEY, group: null, items: ungrouped }]
      : groupSections;
  }, [hosts, groups]);

  const requestDeleteHost = useCallback(
    (id: string) => {
      if (confirmingDeleteHost !== id) {
        setConfirmingDeleteHost(id);
        setConfirmingDeleteGroup(null);
        return;
      }
      setConfirmingDeleteHost(null);
      void deleteHost(id)
        .catch(failAction)
        .finally(() => void load());
    },
    [confirmingDeleteHost, failAction, load],
  );

  const requestDeleteGroup = useCallback(
    (id: string) => {
      if (confirmingDeleteGroup !== id) {
        setConfirmingDeleteGroup(id);
        setConfirmingDeleteHost(null);
        return;
      }
      setConfirmingDeleteGroup(null);
      void deleteHostGroup(id)
        .catch(failAction)
        .finally(() => void load());
    },
    [confirmingDeleteGroup, failAction, load],
  );

  const renderHostCard = (host: Host) => {
    const accent = host.color ? `var(--tyba-${host.color})` : undefined;
    return (
      <div
        key={host.id}
        role="button"
        tabIndex={0}
        onDoubleClick={() => onConnect(host)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onConnect(host);
        }}
        className="group relative flex flex-col justify-between gap-3 overflow-hidden rounded-lg border border-tyba-border bg-tyba-text/[.02] p-3 transition-colors hover:border-tyba-border-strong hover:bg-tyba-text/[.04]"
      >
        {accent && (
          <span
            className="absolute inset-y-0 left-0 w-0.5"
            style={{ background: accent }}
          />
        )}
        <div className="flex items-start gap-2.5">
          <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-tyba-text/[.04] text-tyba-text-muted">
            <HardDrives size={16} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <span
                className="size-1.5 shrink-0 rounded-full"
                style={{ background: accent ?? "var(--tyba-text-faint)" }}
              />
              <span className="truncate text-[13px] font-semibold text-tyba-text">
                {host.alias}
              </span>
            </div>
            <div className="truncate font-mono text-[11px] text-tyba-text-muted">
              {host.username ? `${host.username}@` : ""}
              {host.hostname}
            </div>
            <div className="truncate font-mono text-[10px] text-tyba-text-faint">
              {t("connectionsPort", { port: host.port ?? 22 })}
              {host.proxy_jump ? ` · ${host.proxy_jump}` : ""}
              {host.tunnels.length > 0
                ? ` · ${t("connectionsTunnels", { count: host.tunnels.length })}`
                : ""}
            </div>
          </div>
          <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
            {confirmingDeleteHost === host.id ? (
              <button
                type="button"
                onClick={() => requestDeleteHost(host.id)}
                className="rounded-[3px] bg-tyba-red px-1.5 py-0.5 text-[10px] font-medium text-white hover:bg-tyba-red/90"
              >
                {t("connectionsDeleteHostConfirm")}
              </button>
            ) : (
              <>
                <PanelAction
                  label={t("connectionsEditHost")}
                  onClick={() => setHostDialog({ mode: "edit", host })}
                >
                  <PencilSimple size={13} />
                </PanelAction>
                <PanelAction
                  label={t("connectionsDeleteHost")}
                  destructive
                  onClick={() => requestDeleteHost(host.id)}
                >
                  <Trash size={13} />
                </PanelAction>
              </>
            )}
          </span>
        </div>
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-[10px] text-tyba-text-faint">
            {host.last_connected_at
              ? t("connectionsLastConnected", {
                  when: formatLastConnected(
                    host.last_connected_at,
                    i18n.language,
                  ),
                })
              : t("connectionsNeverConnected")}
          </span>
          <Button
            size="xs"
            onClick={() => onConnect(host)}
            className="h-6 shrink-0 rounded-[4px] px-2 text-[11px]"
          >
            <Plug size={11} />
            {t("connectionsConnect")}
          </Button>
        </div>
      </div>
    );
  };

  const renderSection = (section: {
    key: string;
    group: HostGroup | null;
    items: Host[];
  }) => {
    const isCollapsed = collapsed[section.key] ?? false;
    const accent = section.group?.color
      ? `var(--tyba-${section.group.color})`
      : "var(--tyba-text-faint)";
    return (
      <div key={section.key} className="group/section flex flex-col gap-2.5">
        <div className="flex h-6 items-center gap-2">
          <button
            type="button"
            onClick={() =>
              setCollapsed((prev) => ({ ...prev, [section.key]: !isCollapsed }))
            }
            className="flex min-w-0 items-center gap-1.5 text-left transition-colors hover:text-tyba-text"
          >
            {isCollapsed ? (
              <CaretRight size={10} className="shrink-0 text-tyba-text-faint" />
            ) : (
              <CaretDown size={10} className="shrink-0 text-tyba-text-faint" />
            )}
            <span
              className="size-1.5 shrink-0 rounded-full"
              style={{ background: accent }}
            />
            <span className="min-w-0 truncate text-[11px] font-medium uppercase tracking-[0.1em] text-tyba-text-muted">
              {section.group ? section.group.name : t("connectionsNoGroup")}
            </span>
            <span className="shrink-0 rounded-full bg-tyba-text/[.05] px-1.5 font-mono text-[10px] text-tyba-text-faint">
              {section.items.length}
            </span>
          </button>
          {onConnectGroup && section.items.length > 1 && (
            <Button
              size="xs"
              variant="outline"
              onClick={() => onConnectGroup(section.group, section.items)}
              className="h-5 shrink-0 rounded-[4px] px-1.5 text-[10px] text-tyba-text-muted"
            >
              <Plug size={10} />
              {t("connectionsConnectGroup", { count: section.items.length })}
            </Button>
          )}
          {section.group && (
            <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover/section:opacity-100">
              {confirmingDeleteGroup === section.group.id ? (
                <button
                  type="button"
                  onClick={() => requestDeleteGroup(section.group!.id)}
                  className="rounded-[3px] bg-tyba-red px-1.5 py-0.5 text-[10px] font-medium text-white hover:bg-tyba-red/90"
                >
                  {t("connectionsDeleteGroupConfirm")}
                </button>
              ) : (
                <>
                  <PanelAction
                    label={t("connectionsEditGroup")}
                    onClick={() =>
                      setGroupDialog({ mode: "edit", group: section.group! })
                    }
                  >
                    <PencilSimple size={13} />
                  </PanelAction>
                  <PanelAction
                    label={t("connectionsDeleteGroup")}
                    destructive
                    onClick={() => requestDeleteGroup(section.group!.id)}
                  >
                    <Trash size={13} />
                  </PanelAction>
                </>
              )}
            </span>
          )}
          <span className="h-px min-w-0 flex-1 bg-tyba-border" />
        </div>
        {!isCollapsed && (
          <div className="grid gap-2.5 [grid-template-columns:repeat(auto-fill,minmax(260px,1fr))]">
            {section.items.map(renderHostCard)}
          </div>
        )}
      </div>
    );
  };

  const isLoading = hosts === null || groups === null;
  const isEmpty = !isLoading && hosts.length === 0 && groups.length === 0;

  return (
    <div className="flex min-h-0 flex-1 justify-center overflow-y-auto">
      <div className="flex w-full max-w-5xl flex-col px-6 pt-5 pb-8">
        <div className="flex items-center justify-between pb-3">
          <span className="tyba-label">{t("connectionsTitle")}</span>
          <div className="flex items-center gap-1.5">
            <Button
              size="sm"
              variant="outline"
              onClick={() => setGroupDialog({ mode: "create" })}
              className="h-6 rounded-[4px] px-2 text-[11px] text-tyba-text-muted"
            >
              <FolderPlus size={12} />
              {t("connectionsNewGroup")}
            </Button>
            <Button
              size="sm"
              onClick={() => setHostDialog({ mode: "create" })}
              className="h-6 rounded-[4px] px-2 text-[11px]"
            >
              <Plus size={12} />
              {t("connectionsNewHost")}
            </Button>
          </div>
        </div>

        {loadError !== null ? (
          <div className="flex flex-col items-center gap-3 px-3 py-6 text-center">
            <p className="text-xs text-tyba-text-faint">
              {t("connectionsLoadError")}
            </p>
            <p className="max-w-full truncate font-mono text-[10px] text-tyba-text-faint">
              {loadError}
            </p>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void load()}
              className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
            >
              {t("retry")}
            </Button>
          </div>
        ) : isLoading ? (
          <div className="flex flex-col gap-1 p-2">
            {[0, 1, 2].map((i) => (
              <div
                key={i}
                className="h-9 animate-pulse rounded-md bg-tyba-text/[.03]"
              />
            ))}
          </div>
        ) : isEmpty ? (
          <div className="flex flex-col items-center gap-2 px-3 py-10 text-center">
            <Plugs size={22} className="text-tyba-text-faint" />
            <p className="text-xs text-tyba-text-faint">
              {t("connectionsEmpty")}
            </p>
            <p className="max-w-xs text-[11px] text-tyba-text-faint">
              {t("connectionsEmptyHint")}
            </p>
          </div>
        ) : (
          <div className="flex flex-col gap-5">{sections.map(renderSection)}</div>
        )}
      </div>

      <HostDialog
        state={hostDialog}
        groups={groups ?? []}
        onClose={() => setHostDialog(null)}
        onSaved={() => {
          setHostDialog(null);
          void load();
        }}
      />
      <GroupDialog
        state={groupDialog}
        onClose={() => setGroupDialog(null)}
        onSaved={() => {
          setGroupDialog(null);
          void load();
        }}
      />

      <ToastProvider swipeDirection="right" duration={Infinity}>
        {actionError !== null && (
          <Toast
            onOpenChange={(nextOpen) => {
              if (!nextOpen) setActionError(null);
            }}
          >
            <div className="flex items-start gap-2">
              <Warning
                aria-hidden="true"
                size={16}
                weight="fill"
                className="mt-0.5 shrink-0 text-tyba-amber"
              />
              <div className="min-w-0 flex-1">
                <ToastTitle>{t("connectionsActionFailed")}</ToastTitle>
                <ToastDescription>
                  <span className="block break-words font-mono text-xs">
                    {actionError}
                  </span>
                </ToastDescription>
              </div>
            </div>
          </Toast>
        )}
        <ToastViewport className="bottom-4 top-auto" />
      </ToastProvider>
    </div>
  );
}

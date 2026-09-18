import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CheckCircle,
  CircleNotch,
  Info,
  Plug,
  Plus,
  Prohibit,
  Pulse,
  Trash,
  Warning,
  WarningOctagon,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { HostAuthField } from "@/components/HostAuthField";
import { RiskyTunnelConfirm } from "@/components/RiskyTunnelConfirm";
import { connectionTestCopy, type TestCopy, type TestTone } from "@/lib/canoFailure";
import { translateError } from "@/lib/errors";
import {
  NO_GROUP_VALUE,
  applyInputToHost,
  emptyHostForm,
  formToInput,
  hostToForm,
  usernameNeedsWarning,
  type HostFormValues,
} from "@/lib/hostForm";
import {
  createHost,
  testHostConnection,
  updateHost,
  type Host,
  type HostGroup,
  type Tunnel,
  type TunnelKind,
} from "@/lib/ipc";
import { BLANK_TUNNEL, addedRiskyTunnels } from "@/lib/tunnels";

export const CONNECTION_COLORS = [
  "green",
  "amber",
  "magenta",
  "violet",
  "blue",
  "cyan",
  "red",
];

export type HostDialogState =
  | { mode: "create" }
  | { mode: "edit"; host: Host }
  | null;

export function ColorPicker({
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

export function FormField({
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

type TestState =
  | { state: "idle" }
  | { state: "running" }
  | { state: "done"; copy: TestCopy }
  | { state: "error"; message: string };

const TEST_TONE: Record<TestTone, { box: string; icon: React.ReactNode }> = {
  ok: {
    box: "border-tyba-green/40 bg-tyba-green/10 text-tyba-green",
    icon: <CheckCircle size={13} weight="fill" />,
  },
  info: {
    box: "border-tyba-blue/40 bg-tyba-blue/10 text-tyba-blue",
    icon: <Info size={13} weight="fill" />,
  },
  warning: {
    box: "border-tyba-amber/40 bg-tyba-amber/10 text-tyba-amber",
    icon: <Warning size={13} weight="fill" />,
  },
  danger: {
    box: "border-tyba-red/40 bg-tyba-red/10 text-tyba-red",
    icon: <WarningOctagon size={13} weight="fill" />,
  },
};

function TestResult({ test }: { test: TestState }) {
  const { t } = useTranslation();
  if (test.state === "idle") return null;
  if (test.state === "running") {
    return (
      <div className="flex items-start gap-2 rounded-[4px] border border-tyba-border p-2 text-[12px] text-tyba-text-muted">
        <CircleNotch size={13} className="mt-0.5 shrink-0 animate-spin" weight="bold" />
        <span>{t("hostTestRunning")}</span>
      </div>
    );
  }
  if (test.state === "error") {
    return (
      <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
        {test.message}
      </div>
    );
  }
  const { copy } = test;
  const tone = TEST_TONE[copy.tone];
  return (
    <div
      role="status"
      className={`flex items-start gap-2 rounded-[4px] border p-2 text-[12px] ${tone.box}`}
    >
      <span className="mt-0.5 shrink-0">{tone.icon}</span>
      <div className="min-w-0 flex-1">
        <p className="break-words">{t(copy.titleKey, copy.params)}</p>
        {copy.hintKey && (
          <p className="mt-0.5 text-[11px] text-tyba-text-muted">
            {t(copy.hintKey, copy.params)}
          </p>
        )}
        {copy.params.detail && (
          <p className="mt-1 break-words font-mono text-[10px] text-tyba-text-faint">
            {copy.params.detail}
          </p>
        )}
      </div>
    </div>
  );
}

export function HostFormDialog({
  state,
  groups,
  onClose,
  onSaved,
}: {
  state: HostDialogState;
  groups: HostGroup[];
  onClose: () => void;
  onSaved: (host: Host) => void;
}) {
  const { t, i18n } = useTranslation();
  const [values, setValues] = useState<HostFormValues>(emptyHostForm());
  // Fora de `HostFormValues` de propósito: a chave não passa por validação nem
  // pelo teste de conexão, e entra no `HostInput` só na hora de gravar.
  const [integrationEnabled, setIntegrationEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tunnelError, setTunnelError] = useState<string | null>(null);
  const [confirmRisky, setConfirmRisky] = useState<Tunnel[] | null>(null);
  const [test, setTest] = useState<TestState>({ state: "idle" });
  // O teste leva até 30 s; resposta de um teste antigo (formulário mudou ou
  // diálogo fechou) não pode aparecer como se fosse do atual.
  const testRun = useRef(0);

  useEffect(() => {
    if (state === null) return;
    setValues(state.mode === "edit" ? hostToForm(state.host) : emptyHostForm());
    // Host gravado antes desta versão chega sem o campo, e ausente é LIGADA —
    // o mesmo default do serde no core (regra 11).
    setIntegrationEnabled(
      state.mode === "edit" ? (state.host.integration_enabled ?? true) : true,
    );
    setError(null);
    setBusy(false);
    setTunnelError(null);
    setConfirmRisky(null);
    testRun.current += 1;
    setTest({ state: "idle" });
  }, [state]);

  if (state === null) return null;

  const update = (patch: Partial<HostFormValues>) => {
    testRun.current += 1;
    setTest({ state: "idle" });
    setValues((v) => ({ ...v, ...patch }));
  };

  const groupOptions = [
    { value: NO_GROUP_VALUE, label: t("connectionsNoGroup") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ];

  const editTunnels = (next: (tunnels: Tunnel[]) => Tunnel[]) => {
    setTunnelError(null);
    setConfirmRisky(null);
    testRun.current += 1;
    setTest({ state: "idle" });
    setValues((v) => ({ ...v, tunnels: next(v.tunnels) }));
  };

  const patchTunnel = (index: number, patch: Partial<Tunnel>) =>
    editTunnels((list) =>
      list.map((tn, i) => (i === index ? { ...tn, ...patch } : tn)),
    );

  const validated = () => {
    const result = formToInput(values);
    if (!result.ok) {
      if (result.field === "tunnels") {
        setTunnelError(t(result.errorKey));
      } else {
        setError(t(result.errorKey));
      }
      return null;
    }
    setTunnelError(null);
    setError(null);
    return result.input;
  };

  const runTest = async () => {
    const input = validated();
    if (!input) return;
    const run = ++testRun.current;
    setTest({ state: "running" });
    try {
      const outcome = await testHostConnection(input);
      if (run !== testRun.current) return;
      setTest({ state: "done", copy: connectionTestCopy(outcome, i18n.language) });
    } catch (e) {
      if (run !== testRun.current) return;
      setTest({ state: "error", message: translateError(e, t) });
    }
  };

  const save = async (confirmed: boolean) => {
    const input = validated();
    if (!input) return;
    const tunnels = input.tunnels ?? [];
    setValues((v) => ({ ...v, tunnels }));
    setBusy(true);
    try {
      // A chave entra aqui, e não em `formToInput`: `applyInputToHost` copia
      // campo a campo e devolveria o valor GRAVADO, não o que está na tela.
      const saved =
        state.mode === "create"
          ? await createHost(
              { ...input, integration_enabled: integrationEnabled },
              confirmed,
            )
          : await updateHost(
              {
                ...applyInputToHost(state.host, input),
                integration_enabled: integrationEnabled,
              },
              confirmed,
            );
      onSaved(saved);
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
      <DialogContent className="max-h-[90vh] max-w-[520px] overflow-y-auto border-tyba-border-strong bg-tyba-surface">
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
              onChange={(e) => update({ alias: e.target.value })}
            />
          </FormField>
          <FormField label={t("hostFieldHostname")} htmlFor="host-hostname">
            <Input
              id="host-hostname"
              value={values.hostname}
              placeholder={t("hostFieldHostnamePlaceholder")}
              onChange={(e) => update({ hostname: e.target.value })}
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
              onChange={(e) => update({ port: e.target.value })}
            />
          </FormField>
          <FormField label={t("hostFieldUsername")} htmlFor="host-username">
            <Input
              id="host-username"
              value={values.username}
              placeholder={t("hostFieldUsernamePlaceholder")}
              aria-describedby={
                usernameNeedsWarning(values.username)
                  ? "host-username-warning"
                  : undefined
              }
              onChange={(e) => update({ username: e.target.value })}
            />
          </FormField>
        </div>

        {usernameNeedsWarning(values.username) && (
          <p
            id="host-username-warning"
            className="flex items-start gap-1.5 text-[11px] text-tyba-amber"
          >
            <Warning size={12} weight="fill" className="mt-0.5 shrink-0" />
            {t("hostUsernameUppercase")}
          </p>
        )}

        <FormField label={t("hostAuthMethod")} htmlFor="host-auth">
          <div id="host-auth">
            <HostAuthField
              method={values.auth_method}
              identityFile={values.identity_file}
              agentKey={values.agent_key}
              savedAlias={state.mode === "edit" ? state.host.alias : null}
              onMethodChange={(auth_method) => update({ auth_method })}
              onIdentityFileChange={(identity_file) => update({ identity_file })}
              onAgentKeyChange={(agent_key) => update({ agent_key })}
            />
          </div>
        </FormField>

        <FormField label={t("hostFieldProxyJump")} htmlFor="host-proxy-jump">
          <Input
            id="host-proxy-jump"
            value={values.proxy_jump}
            placeholder={t("hostFieldProxyJumpPlaceholder")}
            onChange={(e) => update({ proxy_jump: e.target.value })}
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
                  onClick={() =>
                    editTunnels((list) => list.filter((_, j) => j !== i))
                  }
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
              onClick={() => editTunnels((list) => [...list, BLANK_TUNNEL])}
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
              onChange={(group_id) => update({ group_id })}
            />
          </FormField>
          <FormField label={t("hostFieldColor")} htmlFor="host-color">
            <ColorPicker
              value={values.color}
              onChange={(color) => update({ color })}
              noColorLabel={t("noColor")}
            />
          </FormField>
        </div>

        <FormField
          label={t("hostFieldIntegration")}
          htmlFor="host-integration"
          hint={t("hostFieldIntegrationHint")}
        >
          <div className="flex items-center justify-between gap-3 rounded-[6px] border border-tyba-border px-3 py-2">
            <span className="text-[12px] text-tyba-text">
              {t("hostFieldIntegration")}
            </span>
            <Switch
              id="host-integration"
              aria-label={t("hostFieldIntegration")}
              checked={integrationEnabled}
              onCheckedChange={setIntegrationEnabled}
            />
          </div>
        </FormField>

        <FormField label={t("hostFieldNotes")} htmlFor="host-notes">
          {/* Prosa: religa o que o `Textarea` base desliga (regra 29). */}
          <Textarea
            id="host-notes"
            rows={2}
            autoCapitalize="sentences"
            autoCorrect="on"
            spellCheck
            value={values.notes}
            placeholder={t("hostFieldNotesPlaceholder")}
            onChange={(e) => update({ notes: e.target.value })}
          />
        </FormField>

        <TestResult test={test} />

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
          <div className="flex items-center justify-between gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => void runTest()}
              disabled={busy || test.state === "running"}
            >
              {test.state === "running" ? (
                <CircleNotch size={12} className="animate-spin" weight="bold" />
              ) : (
                <Pulse size={12} />
              )}
              {t("hostTestAction")}
            </Button>
            <div className="flex gap-2">
              <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
                {t("cancel")}
              </Button>
              <Button size="sm" onClick={() => void save(false)} disabled={busy}>
                {busy ? t("connectionsSaving") : t("connectionsSave")}
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

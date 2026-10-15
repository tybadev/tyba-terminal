import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Plus, Stack, Warning, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { LaunchCanvas } from "@/components/LaunchCanvas";
import * as ipc from "@/lib/ipc";
import type { LaunchSlot, SlotId, SlotNode, SplitKind } from "@/lib/ipc";
import { countPanes, findPaneOfSlot, removePane, slotIds, splitPane } from "@/lib/slotTree";

export interface LaunchConfigDraftState {
  id?: ipc.LaunchConfigId;
  name: string;
  repoRoot: string;
  slots: LaunchSlot[];
  tabs: ipc.LaunchConfigTab[];
}

const uid = () => crypto.randomUUID();
const CLAUDE_MODELS = ["opus", "sonnet", "haiku"] as const;

/**
 * O editor de launch config em tela cheia (quadro "Launch config · editar"):
 * abre no lugar do shell, com o nome e a pasta na barra de título, os panes
 * desenhados como panes do TYBA e, à direita, o que roda no pane escolhido.
 */
export function LaunchConfigScreen({
  draft,
  onClose,
  onSaved,
  onOpen,
}: {
  draft: LaunchConfigDraftState;
  onClose: () => void;
  onSaved: (saved: ipc.SavedLaunchConfig) => void;
  /** "Salvar e abrir": aplica a config logo depois de salvar. */
  onOpen: (id: ipc.LaunchConfigId) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(draft.name);
  const [repoRoot, setRepoRoot] = useState(draft.repoRoot);
  const [slots, setSlots] = useState<LaunchSlot[]>(draft.slots);
  const [tabs, setTabs] = useState<ipc.LaunchConfigTab[]>(draft.tabs);
  const [activeTab, setActiveTab] = useState(0);
  const [selected, setSelected] = useState<SlotId | null>(
    draft.tabs[0] ? (slotIds(draft.tabs[0].root)[0] ?? null) : null,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [hosts, setHosts] = useState<ipc.Host[]>([]);

  useEffect(() => {
    void ipc
      .listHosts()
      .then(setHosts)
      .catch(() => setHosts([]));
  }, []);

  // Esc fecha, como as outras telas cheias — menos quando o foco está num campo.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const el = document.activeElement;
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return;
      onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const tab = tabs[activeTab];
  const slot = useMemo(() => slots.find((s) => s.id === selected) ?? null, [slots, selected]);

  const updateRoot = (root: SlotNode) =>
    setTabs((prev) => prev.map((tb, i) => (i === activeTab ? { ...tb, root } : tb)));
  const updateSlot = (patch: Partial<LaunchSlot>) => {
    if (!slot) return;
    setSlots((prev) => prev.map((s) => (s.id === slot.id ? { ...s, ...patch } : s)));
  };
  const blankSlot = (): LaunchSlot => ({
    id: uid(),
    name: nextSlotName(slots),
    kind: { type: "shell" },
    cwd_rel: null,
    isolate: false,
    initial_prompt: null,
    model: null,
    run_on_open: false,
  });

  const doSplit = (kind: SplitKind) => {
    if (!tab || !selected) return;
    const pane = findPaneOfSlot(tab.root, selected);
    if (!pane) return;
    const fresh = blankSlot();
    setSlots((prev) => [...prev, fresh]);
    updateRoot(splitPane(tab.root, pane, kind, uid(), fresh.id, uid()));
    setSelected(fresh.id);
  };
  const duplicate = () => {
    if (!tab || !slot) return;
    const pane = findPaneOfSlot(tab.root, slot.id);
    if (!pane) return;
    const copy: LaunchSlot = { ...slot, id: uid(), name: nextSlotName(slots) };
    setSlots((prev) => [...prev, copy]);
    updateRoot(splitPane(tab.root, pane, "v", uid(), copy.id, uid()));
    setSelected(copy.id);
  };
  const doRemove = () => {
    if (!tab || !selected) return;
    const pane = findPaneOfSlot(tab.root, selected);
    if (!pane || countPanes(tab.root) < 2) return;
    const next = removePane(tab.root, pane);
    if (!next) return;
    setSlots((prev) => prev.filter((s) => s.id !== selected));
    updateRoot(next);
    setSelected(slotIds(next)[0] ?? null);
  };
  const addTab = () => {
    const fresh = blankSlot();
    setSlots((prev) => [...prev, fresh]);
    setTabs((prev) => [
      ...prev,
      { id: uid(), title: null, root: { type: "leaf", id: uid(), slot_id: fresh.id } },
    ]);
    setActiveTab(tabs.length);
    setSelected(fresh.id);
  };
  const removeTab = (index: number) => {
    if (tabs.length < 2) return;
    const orphaned = new Set(slotIds(tabs[index].root));
    const nextTabs = tabs.filter((_, i) => i !== index);
    setTabs(nextTabs);
    setSlots((prev) => prev.filter((s) => !orphaned.has(s.id)));
    const nextIndex = Math.min(index, nextTabs.length - 1);
    setActiveTab(nextIndex);
    setSelected(slotIds(nextTabs[nextIndex].root)[0] ?? null);
  };
  const pickFolder = async () => {
    const dir = await openFileDialog({ directory: true, multiple: false });
    if (typeof dir === "string") setRepoRoot(dir);
  };

  const save = async (open: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const saved = await ipc.saveLaunchConfig({ name, repo_root: repoRoot, slots, tabs }, draft.id);
      if (saved.secret_warnings.length > 0 && warnings.length === 0) {
        setWarnings(saved.secret_warnings);
        setBusy(false);
        return;
      }
      onSaved(saved);
      onClose();
      if (open) onOpen(saved.id);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const panes = slots.length;
  const agents = slots.filter((s) => s.kind.type === "agent");
  const isolated = agents.filter((s) => s.isolate).length;
  const canRemove = tab != null && countPanes(tab.root) > 1;
  const isClaude = slot?.kind.type === "agent" && slot.kind.runner === "claude_code";
  const runs: { id: string; label: string; on: boolean; pick: () => void; disabled?: boolean }[] = slot
    ? [
        { id: "shell", label: "shell", on: slot.kind.type === "shell", pick: () => updateSlot({ kind: { type: "shell" }, model: null }) },
        { id: "claude", label: "claude", on: isClaude, pick: () => updateSlot({ kind: { type: "agent", runner: "claude_code" }, run_on_open: false }) },
        {
          id: "codex",
          label: "codex",
          on: slot.kind.type === "agent" && slot.kind.runner === "codex",
          pick: () => updateSlot({ kind: { type: "agent", runner: "codex" }, model: null, run_on_open: false }),
        },
        {
          id: "ssh",
          label: "ssh",
          on: slot.kind.type === "ssh",
          disabled: hosts.length === 0,
          pick: () =>
            updateSlot({
              kind: { type: "ssh", host_id: hosts[0].id },
              isolate: false,
              cwd_rel: null,
              initial_prompt: null,
              model: null,
              run_on_open: false,
            }),
        },
      ]
    : [];

  const segment = (items: { id: string; label: string; on: boolean; pick: () => void; disabled?: boolean }[]) => (
    <span className="inline-flex rounded-[7px] bg-tyba-raised p-0.5 shadow-[inset_0_0_0_1px_var(--tyba-border-field)]">
      {items.map((item) => (
        <button
          key={item.id}
          onClick={item.pick}
          disabled={item.disabled}
          aria-pressed={item.on}
          className={`rounded-[5px] px-2.5 py-1 text-[12px] transition-colors disabled:opacity-40 ${
            item.on ? "bg-tyba-text/[.08] text-tyba-text" : "text-tyba-text-faint hover:text-tyba-text-muted"
          }`}
        >
          {item.label}
        </button>
      ))}
    </span>
  );
  const field = (label: string, control: React.ReactNode, hint?: string) => (
    <div className="flex flex-col gap-1.5 border-b border-tyba-border-soft px-4 py-[11px]">
      <span className="text-[11px] uppercase tracking-[.06em] text-tyba-text-faint">{label}</span>
      {control}
      {hint && <span className="text-[11px] leading-[15px] text-tyba-text-faint">{hint}</span>}
    </div>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-tyba-sunken">
      <header className="flex h-[46px] shrink-0 items-center gap-3 border-b border-tyba-border bg-tyba-raised px-[18px] text-[12px]">
        <Stack size={16} className="shrink-0 text-tyba-violet" />
        <span className="text-[14px] font-medium text-tyba-text">{t("launchConfigTitle")}</span>
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("launchConfigNamePlaceholder")}
          autoFocus={!draft.id}
          className="h-7 w-56 text-[13px]"
        />
        <button
          onClick={() => void pickFolder()}
          className="flex min-w-0 items-center gap-1.5 text-tyba-text-muted hover:text-tyba-text"
        >
          <FolderOpen size={14} className="shrink-0" />
          <span className="truncate font-mono">{repoRoot || t("launchConfigNoFolder")}</span>
          <span className="shrink-0 text-tyba-blue">{repoRoot ? t("launchConfigChangeFolder") : t("launchConfigPickFolder")}</span>
        </button>
        <span className="flex-1" />
        <Button variant="ghost" size="sm" onClick={onClose}>
          {t("cancel")}
        </Button>
        <Button variant="outline" size="sm" onClick={() => void save(false)} disabled={busy || !name.trim() || !repoRoot}>
          {warnings.length > 0 ? t("launchSaveAnyway") : t("launchConfigSave")}
        </Button>
        <Button size="sm" onClick={() => void save(true)} disabled={busy || !name.trim() || !repoRoot}>
          {t("launchConfigSaveOpen")}
        </Button>
      </header>

      {(warnings.length > 0 || error) && (
        <div className="flex items-start gap-2 border-b border-tyba-border bg-tyba-raised px-[18px] py-2 text-[12px]">
          <Warning size={15} className={`mt-0.5 shrink-0 ${error ? "text-tyba-red" : "text-tyba-amber"}`} />
          <span className="text-tyba-text">
            {error ?? t("launchSecretWarning", { slots: warnings.join(", ") })}
          </span>
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center gap-1.5 px-[18px] pt-2.5 pb-2 text-[12px]">
            {tabs.map((item, index) => (
              <div
                key={item.id}
                className={`group flex items-center gap-1.5 rounded-[6px] px-2.5 py-1 ${
                  index === activeTab
                    ? "bg-tyba-raised text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-border-field)]"
                    : "text-tyba-text-faint hover:text-tyba-text-muted"
                }`}
              >
                <button
                  onClick={() => {
                    setActiveTab(index);
                    setSelected(slotIds(item.root)[0] ?? null);
                  }}
                >
                  {item.title?.trim() ? item.title : t("launchTabLabel", { n: index + 1 })}
                  <span className="text-tyba-text-faint"> · {t("launchPanesCount", { count: countPanes(item.root) })}</span>
                </button>
                {tabs.length > 1 && (
                  <button
                    aria-label={t("launchRemoveTab")}
                    onClick={() => removeTab(index)}
                    className="opacity-0 transition-opacity group-hover:opacity-100 hover:text-tyba-red"
                  >
                    <X size={11} />
                  </button>
                )}
              </div>
            ))}
            <button onClick={addTab} className="flex items-center gap-1 px-2 py-1 text-tyba-text-faint hover:text-tyba-text">
              <Plus size={12} />
              {t("launchAddTabShort")}
            </button>
            <span className="flex-1" />
            <span className="text-[11px] text-tyba-text-faint">{t("launchPickPaneHint")}</span>
          </div>
          {tab && (
            <div className="min-h-0 flex-1 px-[18px] pb-[18px]">
              <LaunchCanvas
                root={tab.root}
                slots={slots}
                selected={selected}
                onSelect={setSelected}
                onChange={updateRoot}
                onSplit={doSplit}
                onRemove={canRemove ? doRemove : undefined}
              />
            </div>
          )}
        </div>

        <aside className="flex w-[330px] shrink-0 flex-col border-l border-tyba-border bg-tyba-raised">
          {slot ? (
            <>
              <div className="flex items-center gap-2 border-b border-tyba-border-soft px-4 py-3 text-[13px] text-tyba-text">
                {t("launchPane")}
                <Input
                  value={slot.name}
                  onChange={(e) => updateSlot({ name: e.target.value })}
                  className="h-7 flex-1 font-mono text-[12px] text-tyba-amber"
                />
              </div>
              {field(t("launchSlotRuns"), segment(runs), slot.kind.type === "agent" ? t("launchAgentRunsHint") : undefined)}
              {isClaude &&
                field(
                  t("launchSlotModel"),
                  segment([
                    { id: "default", label: t("launchModelDefault"), on: !slot.model, pick: () => updateSlot({ model: null }) },
                    ...CLAUDE_MODELS.map((m) => ({
                      id: m,
                      label: m[0].toUpperCase() + m.slice(1),
                      on: slot.model === m,
                      pick: () => updateSlot({ model: m }),
                    })),
                  ]),
                )}
              {slot.kind.type === "ssh" &&
                field(
                  t("launchSlotHost"),
                  <select
                    value={slot.kind.host_id}
                    onChange={(e) => updateSlot({ kind: { type: "ssh", host_id: e.target.value } })}
                    className="tyba-focusable h-8 rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2 font-mono text-[12px] text-tyba-text"
                  >
                    {hosts.map((host) => (
                      <option key={host.id} value={host.id}>
                        {host.alias}
                      </option>
                    ))}
                  </select>,
                  t("launchSlotHostHelp"),
                )}
              {slot.kind.type !== "ssh" && (
                <>
                  {field(
                    t("launchSlotFolder"),
                    <Input
                      value={slot.cwd_rel ?? ""}
                      onChange={(e) => updateSlot({ cwd_rel: e.target.value || null })}
                      placeholder="."
                      className="font-mono text-[12px]"
                    />,
                  )}
                  <div className="grid grid-cols-[1fr_auto] items-center gap-3 border-b border-tyba-border-soft px-4 py-[11px]">
                    <div className="flex flex-col gap-[3px]">
                      <span className="text-[13px] text-tyba-text">{t("launchSlotIsolate")}</span>
                      <span className="text-[11px] leading-[15px] text-tyba-text-faint">{t("launchSlotIsolateHelp")}</span>
                    </div>
                    <Switch checked={slot.isolate} onCheckedChange={(v) => updateSlot({ isolate: v })} />
                  </div>
                  {slot.kind.type === "shell" ? (
                    <>
                      {field(
                        t("launchSlotCommand"),
                        <Input
                          value={slot.initial_prompt ?? ""}
                          onChange={(e) => updateSlot({ initial_prompt: e.target.value || null })}
                          placeholder="bun run dev"
                          className="font-mono text-[12px]"
                        />,
                      )}
                      <div className="grid grid-cols-[1fr_auto] items-center gap-3 border-b border-tyba-border-soft px-4 py-[11px]">
                        <div className="flex flex-col gap-[3px]">
                          <span className="text-[13px] text-tyba-text">{t("launchSlotRunOnOpen")}</span>
                          <span className="text-[11px] leading-[15px] text-tyba-text-faint">{t("launchSlotRunOnOpenHint")}</span>
                        </div>
                        <Switch
                          checked={slot.run_on_open ?? false}
                          onCheckedChange={(v) => updateSlot({ run_on_open: v })}
                        />
                      </div>
                    </>
                  ) : (
                    field(
                      t("launchSlotPrompt"),
                      <Textarea
                        value={slot.initial_prompt ?? ""}
                        onChange={(e) => updateSlot({ initial_prompt: e.target.value || null })}
                        placeholder={t("launchSlotPromptPlaceholder")}
                        rows={4}
                      />,
                      t("launchSlotPromptHelp"),
                    )
                  )}
                </>
              )}
              <span className="flex-1" />
              <div className="flex gap-2 border-t border-tyba-border-soft px-4 py-3">
                <Button variant="ghost" size="sm" onClick={duplicate}>
                  {t("launchDuplicatePane")}
                </Button>
                <Button variant="ghost" size="sm" className="text-tyba-red" onClick={doRemove} disabled={!canRemove}>
                  {t("launchRemovePane")}
                </Button>
              </div>
            </>
          ) : (
            <span className="p-4 text-[12px] text-tyba-text-faint">{t("launchSelectPane")}</span>
          )}
        </aside>
      </div>

      <footer className="flex h-[26px] shrink-0 items-center gap-2.5 border-t border-tyba-border bg-tyba-raised px-[18px] text-[11px] text-tyba-text-faint">
        {t("launchSummary", { tabs: tabs.length, panes, isolated })}
        <span className="flex-1" />
        {t("launchDragHint")}
      </footer>
    </div>
  );
}

function nextSlotName(slots: LaunchSlot[]): string {
  const taken = new Set(slots.map((s) => s.name));
  for (let i = slots.length + 1; i < 100; i += 1) {
    const candidate = `slot-${i}`;
    if (!taken.has(candidate)) return candidate;
  }
  return `slot-${crypto.randomUUID().slice(0, 4)}`;
}

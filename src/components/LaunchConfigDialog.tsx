import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  FolderOpen,
  Robot,
  SplitHorizontal,
  SplitVertical,
  Trash,
  Warning,
} from "@phosphor-icons/react";

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
import { Textarea } from "@/components/ui/textarea";
import { LaunchCanvas } from "@/components/LaunchCanvas";
import * as ipc from "@/lib/ipc";
import type { LaunchSlot, SlotId, SlotNode, SplitKind } from "@/lib/ipc";
import {
  countPanes,
  findPaneOfSlot,
  removePane,
  slotIds,
  splitPane,
} from "@/lib/slotTree";

export interface LaunchConfigDraftState {
  id?: ipc.LaunchConfigId;
  name: string;
  repoRoot: string;
  slots: LaunchSlot[];
  tabs: ipc.LaunchConfigTab[];
}

interface Props {
  draft: LaunchConfigDraftState | null;
  onClose: () => void;
  onSaved: (saved: ipc.SavedLaunchConfig) => void;
}

const uid = () => crypto.randomUUID();

export function LaunchConfigDialog({ draft, onClose, onSaved }: Props) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [repoRoot, setRepoRoot] = useState("");
  const [slots, setSlots] = useState<LaunchSlot[]>([]);
  const [tabs, setTabs] = useState<ipc.LaunchConfigTab[]>([]);
  const [activeTab, setActiveTab] = useState(0);
  const [selected, setSelected] = useState<SlotId | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);

  useEffect(() => {
    if (!draft) return;
    setName(draft.name);
    setRepoRoot(draft.repoRoot);
    setSlots(draft.slots);
    setTabs(draft.tabs);
    setActiveTab(0);
    setSelected(draft.tabs[0] ? slotIds(draft.tabs[0].root)[0] : null);
    setError(null);
    setWarnings([]);
  }, [draft]);

  const tab = tabs[activeTab];
  const slot = useMemo(
    () => slots.find((s) => s.id === selected) ?? null,
    [slots, selected],
  );

  if (!draft) return null;

  const updateRoot = (root: SlotNode) => {
    setTabs((prev) =>
      prev.map((tb, i) => (i === activeTab ? { ...tb, root } : tb)),
    );
  };

  const updateSlot = (patch: Partial<LaunchSlot>) => {
    if (!slot) return;
    setSlots((prev) =>
      prev.map((s) => (s.id === slot.id ? { ...s, ...patch } : s)),
    );
  };

  const doSplit = (kind: SplitKind) => {
    if (!tab || !selected) return;
    const pane = findPaneOfSlot(tab.root, selected);
    if (!pane) return;
    const newSlot: LaunchSlot = {
      id: uid(),
      name: nextSlotName(slots),
      kind: { type: "shell" },
      cwd_rel: null,
      isolate: false,
      initial_prompt: null,
    };
    setSlots((prev) => [...prev, newSlot]);
    updateRoot(splitPane(tab.root, pane, kind, uid(), newSlot.id, uid()));
    setSelected(newSlot.id);
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

  const pickFolder = async () => {
    const dir = await openFileDialog({ directory: true, multiple: false });
    if (typeof dir === "string") setRepoRoot(dir);
  };

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      const saved = await ipc.saveLaunchConfig(
        { name, repo_root: repoRoot, slots, tabs },
        draft.id,
      );
      if (saved.secret_warnings.length > 0 && warnings.length === 0) {
        setWarnings(saved.secret_warnings);
        setBusy(false);
        return;
      }
      onSaved(saved);
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const canRemove = tab != null && countPanes(tab.root) > 1;

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[880px] sm:max-w-[880px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle>
            {draft.id ? t("launchConfigEdit") : t("launchConfigNew")}
          </DialogTitle>
          <DialogDescription>{t("launchConfigHelp")}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-1.5">
          <span className="tyba-label">{t("launchConfigName")}</span>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("launchConfigNamePlaceholder")}
            autoFocus
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <span className="tyba-label">{t("launchConfigFolder")}</span>
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" onClick={pickFolder}>
              <FolderOpen size={14} />
              {t("launchConfigPickFolder")}
            </Button>
            <span className="truncate font-mono text-[11px] text-tyba-text-faint">
              {repoRoot || t("launchConfigNoFolder")}
            </span>
          </div>
        </div>

        <div className="grid grid-cols-[1fr_280px] gap-4">
          <div className="flex flex-col gap-2">
            {tab && (
              <LaunchCanvas
                root={tab.root}
                slots={slots}
                selected={selected}
                onSelect={setSelected}
                onChange={updateRoot}
              />
            )}
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => doSplit("v")}
                disabled={!selected}
              >
                <SplitVertical size={14} />
                {t("launchSplitVertical")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => doSplit("h")}
                disabled={!selected}
              >
                <SplitHorizontal size={14} />
                {t("launchSplitHorizontal")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={doRemove}
                disabled={!canRemove || !selected}
                className="ml-auto text-tyba-red"
              >
                <Trash size={14} />
                {t("launchRemovePane")}
              </Button>
            </div>
          </div>

          <div className="flex flex-col gap-3 border-l border-tyba-border pl-4">
            {slot ? (
              <>
                <div className="flex flex-col gap-1.5">
                  <span className="tyba-label">{t("launchSlotName")}</span>
                  <Input
                    value={slot.name}
                    onChange={(e) => updateSlot({ name: e.target.value })}
                  />
                </div>

                <div className="flex flex-col gap-1.5">
                  <span className="tyba-label">{t("launchSlotRuns")}</span>
                  <div className="flex gap-1.5">
                    <Button
                      variant={slot.kind.type === "shell" ? "default" : "ghost"}
                      size="sm"
                      onClick={() => updateSlot({ kind: { type: "shell" } })}
                    >
                      shell
                    </Button>
                    <Button
                      variant={slot.kind.type === "agent" ? "default" : "ghost"}
                      size="sm"
                      onClick={() =>
                        updateSlot({
                          kind: { type: "agent", runner: "claude_code" },
                        })
                      }
                    >
                      <Robot size={14} />
                      claude
                    </Button>
                    <Button
                      variant={
                        slot.kind.type === "agent" &&
                        slot.kind.runner === "codex"
                          ? "default"
                          : "ghost"
                      }
                      size="sm"
                      onClick={() =>
                        updateSlot({ kind: { type: "agent", runner: "codex" } })
                      }
                    >
                      codex
                    </Button>
                  </div>
                </div>

                <div className="flex flex-col gap-1.5">
                  <span className="tyba-label">{t("launchSlotFolder")}</span>
                  <Input
                    value={slot.cwd_rel ?? ""}
                    onChange={(e) =>
                      updateSlot({ cwd_rel: e.target.value || null })
                    }
                    placeholder="."
                    className="font-mono text-[12px]"
                  />
                </div>

                <label className="flex items-center justify-between gap-2 rounded-md border border-tyba-border p-2">
                  <span className="flex flex-col">
                    <span className="text-[12px] text-tyba-text">
                      {t("launchSlotIsolate")}
                    </span>
                    <span className="text-[11px] text-tyba-text-faint">
                      {t("launchSlotIsolateHelp")}
                    </span>
                  </span>
                  <Switch
                    checked={slot.isolate}
                    onCheckedChange={(v) => updateSlot({ isolate: v })}
                  />
                </label>

                <div className="flex flex-col gap-1.5">
                  <span className="tyba-label">{t("launchSlotPrompt")}</span>
                  <Textarea
                    value={slot.initial_prompt ?? ""}
                    onChange={(e) =>
                      updateSlot({ initial_prompt: e.target.value || null })
                    }
                    placeholder={t("launchSlotPromptPlaceholder")}
                    rows={4}
                  />
                  <span className="text-[11px] text-tyba-text-faint">
                    {t("launchSlotPromptHelp")}
                  </span>
                </div>
              </>
            ) : (
              <span className="text-[12px] text-tyba-text-faint">
                {t("launchSelectPane")}
              </span>
            )}
          </div>
        </div>

        {warnings.length > 0 && (
          <div className="flex items-start gap-2 rounded-md border border-tyba-amber bg-tyba-amber-tint p-2 text-[12px] text-tyba-text">
            <Warning size={15} className="mt-0.5 shrink-0 text-tyba-amber" />
            <span>
              {t("launchSecretWarning", { slots: warnings.join(", ") })}
            </span>
          </div>
        )}

        {error && (
          <div className="rounded-md border border-tyba-red bg-tyba-red-tint p-2 text-[12px] text-tyba-text">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            {t("cancel")}
          </Button>
          <Button onClick={save} disabled={busy || !name.trim() || !repoRoot}>
            {warnings.length > 0 ? t("launchSaveAnyway") : t("launchConfigSave")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
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

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { BracketsCurly, Plus, Trash } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  clearCommandHistory,
  deleteSnippet,
  getPref,
  listSnippets,
  setPref,
  saveSnippet,
  setHistoryEnabled,
  type Snippet,
} from "../lib/ipc";
import { PROMPT_MODE_PREF_KEY } from "../lib/commandLine";
import { pushToast, toastError } from "../lib/toast";

const HISTORY_PREF_KEY = "pref.commandHistory";


function blankSnippet(): Snippet {
  return {
    id: crypto.randomUUID(),
    name: "",
    command: "",
    description: null,
    tags: [],
    source: "local",
  };
}

export function ShellSettings() {
  const { t } = useTranslation();
  const [historyOn, setHistoryOn] = useState(true);
  const [promptOn, setPromptOn] = useState(false);
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  const [draft, setDraft] = useState<Snippet | null>(null);

  const refresh = useCallback(() => {
    void listSnippets(null)
      .then((found) => setSnippets(found.filter((s) => s.source === "local")))
      .catch(() => setSnippets([]));
  }, []);

  useEffect(() => {
    void getPref(HISTORY_PREF_KEY)
      .then((value) => setHistoryOn(value !== "off"))
      .catch(() => setHistoryOn(true));
    void getPref(PROMPT_MODE_PREF_KEY)
      .then((value) => setPromptOn(value === "on"))
      .catch(() => setPromptOn(false));
    refresh();
  }, [refresh]);

  const toggleHistory = (next: boolean) => {
    setHistoryOn(next);
    void setHistoryEnabled(next).catch((error) => {
      setHistoryOn(!next);
      toastError(t("historyEnabled"), error);
    });
  };

  const togglePrompt = (next: boolean) => {
    setPromptOn(next);
    void setPref(PROMPT_MODE_PREF_KEY, next ? "on" : "off").catch((error) => {
      setPromptOn(!next);
      toastError(t("promptModeTitle"), error);
    });
  };

  const clear = () => {
    void clearCommandHistory(null)
      .then(() => pushToast({ title: t("historyCleared") }))
      .catch((error) => toastError(t("historyClear"), error));
  };

  const persist = () => {
    if (!draft) return;
    void saveSnippet({
      ...draft,
      name: draft.name.trim(),
      command: draft.command.trim(),
      description: draft.description?.trim() || null,
    })
      .then(() => {
        setDraft(null);
        refresh();
      })
      .catch((error) => toastError(t("snippetSave"), error));
  };

  const remove = (id: string) => {
    void deleteSnippet(id)
      .then(refresh)
      .catch((error) => toastError(t("snippetDelete"), error));
  };

  return (
    <section className="mx-auto w-full max-w-lg">
      <span className="tyba-label">{t("promptModeTitle")}</span>
      <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
        {t("promptModeHint")}
      </p>
      <div className="mt-2 flex items-center gap-3 rounded-[6px] border border-tyba-border p-4">
        <span className="min-w-0 flex-1 text-[13px] text-tyba-text">
          {t("promptModeEnabled")}
        </span>
        <Switch
          checked={promptOn}
          onCheckedChange={togglePrompt}
          aria-label={t("promptModeEnabled")}
        />
      </div>
      <p className="pt-2 text-[11px] leading-relaxed text-tyba-amber">
        {t("promptModeEscape")}
      </p>

      <span className="tyba-label mt-6 block">{t("settingsHistory")}</span>
      <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
        {t("historyHint")}
      </p>
      <div className="mt-2 flex items-center gap-3 rounded-[6px] border border-tyba-border p-4">
        <span className="min-w-0 flex-1 text-[13px] text-tyba-text">
          {t("historyEnabled")}
        </span>
        <Button variant="ghost" size="sm" onClick={clear}>
          {t("historyClear")}
        </Button>
        <Switch
          checked={historyOn}
          onCheckedChange={toggleHistory}
          aria-label={t("historyEnabled")}
        />
      </div>

      <span className="tyba-label mt-6 block">{t("settingsSnippets")}</span>
      <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
        {t("snippetsHint")}
      </p>

      <div className="mt-2 divide-y divide-tyba-border overflow-hidden rounded-[6px] border border-tyba-border">
        {snippets.length === 0 && !draft && (
          <p className="px-4 py-3 text-[12px] text-tyba-text-faint">
            {t("snippetsEmpty")}
          </p>
        )}
        {snippets.map((snippet) => (
          <div key={snippet.id} className="flex items-center gap-3 px-4 py-2">
            <BracketsCurly size={15} className="shrink-0 text-tyba-text-muted" />
            <button
              onClick={() => setDraft(snippet)}
              className="min-w-0 flex-1 text-left"
            >
              <span className="block truncate text-[13px] text-tyba-text">
                {snippet.name}
              </span>
              <span className="block truncate font-mono text-[11px] text-tyba-text-faint">
                {snippet.command}
              </span>
            </button>
            <button
              aria-label={t("snippetDelete")}
              onClick={() => remove(snippet.id)}
              className="shrink-0 rounded-[4px] p-1 text-tyba-text-faint hover:bg-tyba-text/[.05] hover:text-tyba-red"
            >
              <Trash size={14} />
            </button>
          </div>
        ))}
      </div>

      {draft ? (
        <div className="mt-2 flex flex-col gap-2 rounded-[6px] border border-tyba-border p-4">
          <input
            autoFocus
            value={draft.name}
            placeholder={t("snippetName")}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            className="rounded-[4px] border border-tyba-border bg-transparent px-2 py-1 text-[13px] text-tyba-text outline-none focus:border-tyba-green/50"
          />
          <textarea
            value={draft.command}
            rows={2}
            placeholder={t("snippetCommand")}
            onChange={(e) => setDraft({ ...draft, command: e.target.value })}
            className="resize-none rounded-[4px] border border-tyba-border bg-transparent px-2 py-1 font-mono text-[12px] text-tyba-text outline-none focus:border-tyba-green/50"
          />
          <input
            value={draft.description ?? ""}
            placeholder={t("snippetDescription")}
            onChange={(e) =>
              setDraft({ ...draft, description: e.target.value })
            }
            className="rounded-[4px] border border-tyba-border bg-transparent px-2 py-1 text-[12px] text-tyba-text outline-none focus:border-tyba-green/50"
          />
          <div className="flex items-center justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={() => setDraft(null)}>
              {t("cancel")}
            </Button>
            <Button size="sm" onClick={persist}>
              {t("snippetSave")}
            </Button>
          </div>
        </div>
      ) : (
        <Button
          variant="ghost"
          size="sm"
          className="mt-2"
          onClick={() => setDraft(blankSnippet())}
        >
          <Plus size={14} />
          {t("snippetNew")}
        </Button>
      )}
    </section>
  );
}

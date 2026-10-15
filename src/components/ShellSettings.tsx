import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  clearCommandHistory,
  deleteSnippet,
  getPref,
  importShellHistory,
  listSnippets,
  commandHistoryTotal,
  onShellHistoryImportProgress,
  scanShellHistorySources,
  setPref,
  saveSnippet,
  setHistoryEnabled,
  type ImportProgress,
  type ImportReport,
  type ShellHistorySource,
  type Snippet,
} from "../lib/ipc";
import { PROMPT_MODE_PREF_KEY, promptModeEnabled } from "../lib/commandLine";
import { PromptModePicker } from "./PromptModePicker";
import { SettingGroup, SettingLine, SettingNote } from "./settings/rows";
import {
  autocompleteAutoOpen,
  autocompleteShowUses,
  setAutocompleteAutoOpen,
  setAutocompleteShowUses,
} from "../lib/autocompletePrefs";
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

/** O miolo da seção Terminal; `lineRows` são as linhas da linha de comando que moram no `SettingsView`. */
export function ShellSettings({ lineRows }: { lineRows?: React.ReactNode }) {
  const { t } = useTranslation();
  const [historyOn, setHistoryOn] = useState(true);
  const [promptOn, setPromptOn] = useState(false);
  const [autoOpen, setAutoOpen] = useState(autocompleteAutoOpen);
  const [showUses, setShowUses] = useState(autocompleteShowUses);
  const [historyTotal, setHistoryTotal] = useState<number | null>(null);
  useEffect(() => {
    void commandHistoryTotal()
      .then(setHistoryTotal)
      .catch(() => {});
  }, []);
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  const [draft, setDraft] = useState<Snippet | null>(null);
  const [importSources, setImportSources] = useState<ShellHistorySource[]>([]);
  const [importing, setImporting] = useState(false);
  const [importProgress, setImportProgress] = useState<ImportProgress | null>(
    null,
  );
  const [importReport, setImportReport] = useState<ImportReport | null>(null);

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
      .then((value) => setPromptOn(promptModeEnabled(value)))
      .catch(() => setPromptOn(true));
    void scanShellHistorySources()
      .then(setImportSources)
      .catch(() => setImportSources([]));
    refresh();
  }, [refresh]);

  useEffect(() => {
    const unlisten = onShellHistoryImportProgress(setImportProgress);
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

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

  const runImport = () => {
    setImporting(true);
    setImportReport(null);
    void importShellHistory()
      .then((report) => {
        setImportReport(report);
        // Reconta: o que entrou já está no banco, e a contagem da fonte só
        // muda quando o usuário digitar mais fora do TYBA.
        void scanShellHistorySources()
          .then(setImportSources)
          .catch(() => {});
      })
      .catch((error) => toastError(t("historyImportFailed"), error))
      .finally(() => {
        setImporting(false);
        setImportProgress(null);
      });
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
    <>
      <SettingGroup title={t("settingsCommandLine")} />
      {lineRows}
      <SettingLine
        label={t("promptModeTitle")}
        hint={t("promptModeHint")}
        below={<PromptModePicker value={promptOn} onChange={togglePrompt} />}
      />
      <SettingNote tone="amber">{t("promptModeEscape")}</SettingNote>

      <SettingGroup title={t("settingsAutocomplete")} />
      <SettingLine label={t("autocompleteAutoOpen")} hint={t("autocompleteAutoOpenHint")}>
        <Switch
          checked={autoOpen}
          onCheckedChange={(c) => {
            setAutoOpen(c);
            setAutocompleteAutoOpen(c);
          }}
        />
      </SettingLine>
      <SettingLine label={t("autocompleteShowUses")} hint={t("autocompleteShowUsesHint")}>
        <Switch
          checked={showUses}
          onCheckedChange={(c) => {
            setShowUses(c);
            setAutocompleteShowUses(c);
          }}
        />
      </SettingLine>

      <SettingGroup
        title={t("settingsHistory")}
        note={historyTotal !== null ? t("historyTotal", { count: historyTotal }) : undefined}
      />
      <SettingLine label={t("historyEnabled")} hint={t("historyHint")}>
        <Switch checked={historyOn} onCheckedChange={toggleHistory} aria-label={t("historyEnabled")} />
      </SettingLine>
      <SettingLine
        label={t("historyImportTitle")}
        hint={
          importSources.length === 0
            ? t("historyImportNoSources")
            : importSources
                .map((source) => t("historyImportFound", { entries: source.entries, path: source.path }))
                .join(" · ")
        }
      >
        {importProgress && (
          <span className="text-[11px] tabular-nums text-tyba-text-faint">
            {importProgress.source} {importProgress.imported}/{importProgress.total}
          </span>
        )}
        <Button variant="ghost" size="sm" onClick={runImport} disabled={importing || importSources.length === 0}>
          {importing ? t("historyImportRunning") : t("historyImportAction")}
        </Button>
      </SettingLine>
      {importReport?.sources.map((outcome) => (
        <SettingNote key={outcome.path} tone={outcome.skipped ? "amber" : "faint"}>
          {outcome.source}:{" "}
          {outcome.skipped
            ? t("historyImportSkipped", { reason: outcome.skipped })
            : t("historyImportDone", { imported: outcome.imported, discarded: outcome.discarded })}
        </SettingNote>
      ))}
      <SettingLine label={t("historyClear")} hint={t("historyClearHint")}>
        <Button variant="ghost" size="sm" className="text-tyba-red" onClick={clear}>
          {t("historyClear")}
        </Button>
      </SettingLine>

      <SettingGroup
        title={t("settingsSnippets")}
        note={
          !draft && (
            <button onClick={() => setDraft(blankSnippet())} className="text-tyba-blue hover:text-tyba-text">
              + {t("snippetNew")}
            </button>
          )
        }
      />
      {snippets.length === 0 && !draft && <SettingNote>{t("snippetsEmpty")}</SettingNote>}
      {snippets.map((snippet) => (
        <div
          key={snippet.id}
          className="grid grid-cols-[160px_1fr_auto_auto] items-center gap-4 border-b border-tyba-border-soft px-[18px] py-[9px] text-[12.5px]"
        >
          <button onClick={() => setDraft(snippet)} className="truncate text-left text-tyba-text">
            {snippet.name}
          </button>
          <button onClick={() => setDraft(snippet)} className="truncate text-left font-mono text-tyba-text-muted">
            {snippet.command}
          </button>
          <span className="text-[11px] text-tyba-text-faint">
            {snippet.source === "repo" ? t("snippetScopeRepo") : t("snippetScopeGlobal")}
          </span>
          <button
            aria-label={t("snippetDelete")}
            onClick={() => remove(snippet.id)}
            className="rounded-[4px] p-1 text-tyba-text-faint hover:bg-tyba-text/[.05] hover:text-tyba-red"
          >
            <Trash size={14} />
          </button>
        </div>
      ))}
      {draft ? (
        <div className="mx-[18px] mt-2 flex flex-col gap-2 rounded-[6px] border border-tyba-border p-4">
          <input
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            autoFocus
            value={draft.name}
            placeholder={t("snippetName")}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            className="rounded-[4px] border border-tyba-border bg-transparent px-2 py-1 text-[13px] text-tyba-text outline-none focus:border-tyba-green/50"
          />
          <textarea
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={draft.command}
            rows={2}
            placeholder={t("snippetCommand")}
            onChange={(e) => setDraft({ ...draft, command: e.target.value })}
            className="resize-none rounded-[4px] border border-tyba-border bg-transparent px-2 py-1 font-mono text-[12px] text-tyba-text outline-none focus:border-tyba-green/50"
          />
          <input
            autoCapitalize="sentences"
            autoCorrect="on"
            spellCheck
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
      ) : null}
    </>
  );
}

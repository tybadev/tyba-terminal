import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PenNib, Plus, Trash, UploadSimple } from "@phosphor-icons/react";

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
import { SettingButton, SettingGroup, SettingLine, SettingNote } from "./settings/rows";
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

/**
 * O miolo da seção Terminal, na ordem do quadro "Configurações (aprovada)":
 * Digitar, O que o TYBA sabe da sessão (`lineRows`, que moram no
 * `SettingsView`), Histórico e Snippets.
 */
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
      <SettingGroup title={t("cfgGroupTyping")}>
        <SettingLine label={t("promptModeTitle")} hint={t("cfgPromptModeHint")}>
          <Switch checked={promptOn} onCheckedChange={togglePrompt} aria-label={t("promptModeTitle")} />
        </SettingLine>
        <SettingLine label={t("cfgAutoOpen")} hint={t("cfgAutoOpenHint")}>
          <Switch
            checked={autoOpen}
            aria-label={t("cfgAutoOpen")}
            onCheckedChange={(c) => {
              setAutoOpen(c);
              setAutocompleteAutoOpen(c);
            }}
          />
        </SettingLine>
        <SettingLine label={t("autocompleteShowUses")}>
          <Switch
            checked={showUses}
            aria-label={t("autocompleteShowUses")}
            onCheckedChange={(c) => {
              setShowUses(c);
              setAutocompleteShowUses(c);
            }}
          />
        </SettingLine>
      </SettingGroup>

      {lineRows && <SettingGroup title={t("cfgGroupSessionKnows")}>{lineRows}</SettingGroup>}

      <SettingGroup
        title={t("cfgGroupHistory")}
        note={historyTotal !== null ? t("historyTotal", { count: historyTotal }) : undefined}
      >
        <SettingLine label={t("cfgHistoryOn")} hint={t("cfgHistoryOnHint")}>
          <Switch checked={historyOn} onCheckedChange={toggleHistory} aria-label={t("cfgHistoryOn")} />
        </SettingLine>
        <SettingLine
          label={t("cfgHistoryImport")}
          hint={
            importSources.length === 0
              ? t("historyImportNoSources")
              : importSources
                  .map((source) => t("historyImportFound", { entries: source.entries, path: source.path }))
                  .join(" · ")
          }
        >
          {importProgress && (
            <span className="text-[12px] tabular-nums text-tyba-text-faint">
              {importProgress.source} {importProgress.imported}/{importProgress.total}
            </span>
          )}
          <SettingButton
            icon={<UploadSimple size={16} />}
            onClick={runImport}
            disabled={importing || importSources.length === 0}
          >
            {importing ? t("historyImportRunning") : t("historyImportAction")}
          </SettingButton>
        </SettingLine>
        {importReport?.sources.map((outcome) => (
          <SettingNote key={outcome.path} tone={outcome.skipped ? "amber" : "faint"}>
            {outcome.source}:{" "}
            {outcome.skipped
              ? t("historyImportSkipped", { reason: outcome.skipped })
              : t("historyImportDone", { imported: outcome.imported, discarded: outcome.discarded })}
          </SettingNote>
        ))}
        <SettingLine label={t("cfgHistoryClear")} hint={t("cfgHistoryClearHint")}>
          <SettingButton tone="danger" icon={<Trash size={16} />} onClick={clear}>
            {t("cfgClear")}
          </SettingButton>
        </SettingLine>
      </SettingGroup>

      <SettingGroup title={t("settingsSnippets")}>
        {snippets.length === 0 && !draft && <SettingNote>{t("snippetsEmpty")}</SettingNote>}
        {snippets.map((snippet) => (
          <SettingLine
            key={snippet.id}
            label={snippet.name}
            hint={<span className="font-mono text-[12.5px] text-tyba-text">{snippet.command}</span>}
          >
            <SettingButton tone="ghost" icon={<PenNib size={16} />} onClick={() => setDraft(snippet)}>
              {t("launchEdit")}
            </SettingButton>
            <SettingButton
              tone="ghost"
              aria-label={t("snippetDelete")}
              className="px-2.5 hover:text-tyba-red"
              onClick={() => remove(snippet.id)}
              icon={<Trash size={16} />}
            />
          </SettingLine>
        ))}
        {draft ? (
          <div className="my-2 flex flex-col gap-2 rounded-[10px] p-4 shadow-[inset_0_0_0_1px_var(--tyba-line)]">
            <input
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              autoFocus
              value={draft.name}
              placeholder={t("snippetName")}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              className="rounded-[6px] border border-tyba-border bg-transparent px-2 py-1 text-[13px] text-tyba-text outline-none focus:border-tyba-border-strong"
            />
            <textarea
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              value={draft.command}
              rows={2}
              placeholder={t("snippetCommand")}
              onChange={(e) => setDraft({ ...draft, command: e.target.value })}
              className="resize-none rounded-[6px] border border-tyba-border bg-transparent px-2 py-1 font-mono text-[12px] text-tyba-text outline-none focus:border-tyba-border-strong"
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
              className="rounded-[6px] border border-tyba-border bg-transparent px-2 py-1 text-[12px] text-tyba-text outline-none focus:border-tyba-border-strong"
            />
            <div className="flex items-center justify-end gap-2">
              <SettingButton tone="ghost" onClick={() => setDraft(null)}>
                {t("cancel")}
              </SettingButton>
              <SettingButton onClick={persist}>{t("snippetSave")}</SettingButton>
            </div>
          </div>
        ) : (
          <SettingLine>
            <SettingButton icon={<Plus size={16} />} onClick={() => setDraft(blankSnippet())}>
              {t("snippetNew")}
            </SettingButton>
          </SettingLine>
        )}
      </SettingGroup>
    </>
  );
}

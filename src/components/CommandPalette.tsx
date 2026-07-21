import { type KeyboardEvent, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Desktop,
  DownloadSimple,
  FileMagnifyingGlass,
  FolderOpen,
  GearSix,
  Globe,
  MagnifyingGlass,
  Moon,
  Palette,
  Plus,
  SidebarSimple,
  Stack,
  Sun,
  TerminalWindow,
  TextAa,
  X,
  GitBranch,
} from "@phosphor-icons/react";

import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import { Kbd, Shortcut } from "@/components/ui/kbd";
import { LANGUAGES, setLanguage } from "../i18n";
import {
  applyTheme,
  DEFAULT_THEME_IDS,
  THEMES,
  type Theme,
  type ThemeMode,
} from "../theme";
import {
  getUiFont,
  setUiFont,
  UI_FONT_LABELS,
  UI_FONTS,
  type UiFont,
} from "../font";
import {
  importThemeCmd,
  listThemes,
  type FileSearchResult,
  type LaunchConfig,
  type LaunchConfigId,
  type Workspace,
  type WorkspaceId,
} from "../lib/ipc";
import { type Bindings } from "../lib/keys";
import { nextPaletteMode, type PaletteMode } from "../lib/paletteMode";
import { fileIcon } from "../lib/fileIcon";
import { toastError } from "../lib/toast";

const THEME_ICONS: Record<ThemeMode, typeof Moon> = {
  dark: Moon,
  light: Sun,
  system: Desktop,
};

const THEME_LABEL_KEYS: Record<ThemeMode, string> = {
  dark: "themeDark",
  light: "themeLight",
  system: "themeSystem",
};

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: PaletteMode;
  onModeChange: (mode: PaletteMode) => void;
  searchFiles: (query: string) => Promise<FileSearchResult>;
  onOpenFile: (rel: string) => void;
  workspaces: Workspace[];
  activeWorkspace: WorkspaceId | null;
  bindings: Bindings;
  theme: ThemeMode;
  onChangeTheme: (mode: ThemeMode) => void;
  onNewSession: () => void;
  onNewWorktreeSession: () => void;
  onNewTab: () => void;
  onCloseActive: () => void;
  onOpenSettings: () => void;
  onTogglePanel: () => void;
  onOpenFiles: () => void;
  onGoToWorkspace: (id: WorkspaceId) => void;
  launchConfigs: LaunchConfig[];
  onApplyLaunchConfig: (id: LaunchConfigId) => void;
  onNewLaunchConfig: () => void;
  onSaveWorkspaceAsLaunchConfig: () => void;
}

export function CommandPalette({
  open,
  onOpenChange,
  mode,
  onModeChange,
  searchFiles,
  onOpenFile,
  workspaces,
  activeWorkspace,
  bindings,
  theme,
  onChangeTheme,
  onNewSession,
  onNewWorktreeSession,
  onNewTab,
  onCloseActive,
  onOpenSettings,
  onTogglePanel,
  onOpenFiles,
  onGoToWorkspace,
  launchConfigs,
  onApplyLaunchConfig,
  onNewLaunchConfig,
  onSaveWorkspaceAsLaunchConfig,
}: Props) {
  const { t, i18n } = useTranslation();
  const [selectableThemes, setSelectableThemes] = useState<Theme[]>([]);
  const [query, setQuery] = useState("");
  const [fileResults, setFileResults] = useState<string[]>([]);
  const [fileTruncated, setFileTruncated] = useState(false);

  useEffect(() => {
    if (!open) return;
    void listThemes()
      .then((all) =>
        setSelectableThemes(
          all.filter((item) => !DEFAULT_THEME_IDS.includes(item.id)),
        ),
      )
      .catch(() => setSelectableThemes([]));
  }, [open]);

  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  useEffect(() => {
    setQuery("");
  }, [mode]);

  useEffect(() => {
    if (!open || mode !== "files") return;
    let alive = true;
    const run = () => {
      void searchFiles(query)
        .then((result) => {
          if (!alive) return;
          setFileResults(result.paths);
          setFileTruncated(result.truncated);
        })
        .catch(() => {
          if (alive) setFileResults([]);
        });
    };
    const timer = setTimeout(run, query ? 120 : 0);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [open, mode, query, searchFiles]);

  const cycleMode = (e: KeyboardEvent) => {
    if (e.key !== "Tab") return;
    e.preventDefault();
    onModeChange(nextPaletteMode(mode, e.shiftKey ? -1 : 1));
  };

  const run = (fn: () => void) => () => {
    onOpenChange(false);
    fn();
  };

  const importTheme = async () => {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: "Tema Tyba (JSON)", extensions: ["json"] }],
    });
    if (typeof path !== "string") return;
    try {
      const imported = await importThemeCmd(path);
      await applyTheme(imported);
    } catch (error) {
      toastError(t("themeImportFailedTitle"), error);
    }
  };

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("commandPalette")}
      description={t("searchCommand")}
      shouldFilter={mode !== "files"}
      showCloseButton={false}
      className="top-28 max-w-[560px] translate-y-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface shadow-2xl"
    >
      <div className="flex items-center gap-1 border-b border-tyba-border px-2 py-1.5">
        {(["actions", "files", "sessions"] as const).map((m) => {
          const Icon =
            m === "actions"
              ? MagnifyingGlass
              : m === "files"
                ? FileMagnifyingGlass
                : TerminalWindow;
          const label =
            m === "actions"
              ? t("actions")
              : m === "files"
                ? t("filesFinderMode")
                : t("sessions");
          const combo =
            m === "actions"
              ? bindings.paletteActions
              : m === "files"
                ? bindings.filesFinder
                : bindings.paletteSessions;
          return (
            <button
              key={m}
              onClick={() => onModeChange(m)}
              className={`flex items-center gap-1.5 rounded-[4px] px-2 py-1 text-[11px] transition-colors ${
                mode === m
                  ? "bg-tyba-text/[.06] text-tyba-text"
                  : "text-tyba-text-faint hover:text-tyba-text-muted"
              }`}
            >
              <Icon size={12} />
              {label}
              <Shortcut combo={combo} className="ml-1" />
            </button>
          );
        })}
        <span className="ml-auto text-[10px] text-tyba-text-faint">
          {t("paletteTabHint")}
        </span>
      </div>
      <CommandInput
        autoFocus
        value={query}
        onValueChange={setQuery}
        onKeyDown={cycleMode}
        placeholder={
          mode === "sessions"
            ? t("searchSessions")
            : mode === "files"
              ? t("filesFinderPlaceholder")
              : t("searchCommand")
        }
      />
      <CommandList>
        <CommandEmpty>{t("noResults")}</CommandEmpty>

        {mode === "files" && fileResults.length > 0 && (
          <CommandGroup
            heading={
              fileTruncated
                ? `${t("filesFinderMode")} · ${t("filesFinderTruncated")}`
                : t("filesFinderMode")
            }
          >
            {fileResults.map((rel) => {
              const name = rel.slice(rel.lastIndexOf("/") + 1);
              const dir = rel.slice(0, rel.lastIndexOf("/"));
              const Icon = fileIcon(name);
              return (
                <CommandItem
                  key={rel}
                  value={rel}
                  onSelect={run(() => onOpenFile(rel))}
                >
                  <Icon size={15} />
                  <span className="truncate">{name}</span>
                  {dir && (
                    <span className="ml-auto min-w-0 truncate font-mono text-[10px] text-tyba-text-faint">
                      {dir}
                    </span>
                  )}
                </CommandItem>
              );
            })}
          </CommandGroup>
        )}

        {mode === "actions" && (
        <CommandGroup heading={t("actions")}>
          <CommandItem onSelect={run(onNewSession)}>
            <Plus size={15} />
            {t("newSession")}
          </CommandItem>
          <CommandItem onSelect={run(onNewLaunchConfig)}>
            <Stack size={15} />
            {t("launchConfigNew")}
          </CommandItem>
          {activeWorkspace && (
            <CommandItem onSelect={run(onSaveWorkspaceAsLaunchConfig)}>
              <Stack size={15} />
              {t("launchSaveWorkspaceAs")}
            </CommandItem>
          )}
          <CommandItem onSelect={run(onNewWorktreeSession)}>
            <GitBranch size={15} />
            {t("worktreeNewSession")}
            <Shortcut combo={bindings.newWorktreeSession} className="ml-auto" />
          </CommandItem>
          <CommandItem onSelect={run(onNewTab)}>
            <Plus size={15} />
            {t("newTab")}
            <Shortcut combo={bindings.newTab} className="ml-auto" />
          </CommandItem>
          {activeWorkspace && (
            <CommandItem onSelect={run(onCloseActive)}>
              <X size={15} />
              {t("closePane")}
              <Shortcut combo={bindings.closePane} className="ml-auto" />
            </CommandItem>
          )}
          <CommandItem onSelect={run(onTogglePanel)}>
            <SidebarSimple size={15} />
            {t("togglePanel")}
            <Shortcut combo={bindings.panel} className="ml-auto" />
          </CommandItem>
          {activeWorkspace && (
            <CommandItem onSelect={run(onOpenFiles)}>
              <FolderOpen size={15} />
              {t("filesPanel")}
              <Shortcut combo={bindings.files} className="ml-auto" />
            </CommandItem>
          )}
          <CommandItem onSelect={run(onOpenSettings)}>
            <GearSix size={15} />
            {t("settings")}
          </CommandItem>
        </CommandGroup>
        )}

        {mode === "actions" && launchConfigs.length > 0 && (
          <CommandGroup heading={t("launchApply")}>
            {launchConfigs.map((config) => (
              <CommandItem
                key={config.id}
                value={`${config.name} ${config.slug} ${config.repo_root}`}
                onSelect={run(() => onApplyLaunchConfig(config.id))}
              >
                <Stack size={15} className="text-tyba-violet" />
                <span className="truncate">{config.name}</span>
                <span className="ml-auto font-mono text-[10px] text-tyba-text-faint">
                  {config.slots.length}{" "}
                  {config.slots.length === 1 ? "pane" : "panes"}
                </span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}

        {mode === "sessions" && workspaces.length > 0 && (
          <>
            <CommandGroup heading={t("sessions")}>
              {workspaces.map((w) => (
                <CommandItem
                  key={w.id}
                  value={`${w.name} ${w.repo_root ?? ""} ${w.id}`}
                  onSelect={run(() => onGoToWorkspace(w.id))}
                >
                  <TerminalWindow
                    size={15}
                    className={
                      w.id === activeWorkspace ? "text-tyba-green" : undefined
                    }
                  />
                  <span className="truncate">{w.name}</span>
                  {w.tabs.length > 0 && (
                    <span className="ml-auto font-mono text-[10px] text-tyba-text-faint">
                      {w.tabs.length} {w.tabs.length === 1 ? "tab" : "tabs"}
                    </span>
                  )}
                </CommandItem>
              ))}
            </CommandGroup>
          </>
        )}

        {mode === "actions" && (
        <>
        <CommandSeparator />
        <CommandGroup heading={t("theme")}>
          {THEMES.filter((m) => m !== theme).map((tm) => {
            const Icon = THEME_ICONS[tm];
            return (
              <CommandItem key={tm} onSelect={run(() => onChangeTheme(tm))}>
                <Icon size={15} />
                {t("switchThemeTo", { theme: t(THEME_LABEL_KEYS[tm]) })}
              </CommandItem>
            );
          })}
          {selectableThemes.map((item) => (
            <CommandItem
              key={item.id}
              value={`theme ${item.name}`}
              onSelect={run(() => void applyTheme(item))}
            >
              <Palette size={15} />
              {t("useTheme", { name: item.name })}
            </CommandItem>
          ))}
          <CommandItem onSelect={run(() => void importTheme())}>
            <DownloadSimple size={15} />
            {t("importTheme")}
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />
        <CommandGroup heading={t("uiFont")}>
          {UI_FONTS.filter((f) => f !== getUiFont()).map((f: UiFont) => (
            <CommandItem key={f} onSelect={run(() => setUiFont(f))}>
              <TextAa size={15} />
              {t("switchFontTo", { font: UI_FONT_LABELS[f] })}
            </CommandItem>
          ))}
        </CommandGroup>

        <CommandSeparator />
        <CommandGroup heading={t("language")}>
          {LANGUAGES.filter((l) => l.code !== i18n.language).map((lang) => (
            <CommandItem
              key={lang.code}
              onSelect={run(() => setLanguage(lang.code))}
            >
              <Globe size={15} />
              {t("switchLanguageTo", { lang: lang.label })}
            </CommandItem>
          ))}
        </CommandGroup>
        </>
        )}
      </CommandList>
      <div className="flex items-center gap-3 border-t border-tyba-border px-3 py-1.5 text-[10px] text-tyba-text-faint">
        <span className="flex items-center gap-1">
          <Kbd>↑</Kbd>
          <Kbd>↓</Kbd>
          {t("hintNavigate")}
        </span>
        <span className="flex items-center gap-1">
          <Kbd>↵</Kbd>
          {t("hintRun")}
        </span>
        <span className="ml-auto flex items-center gap-1">
          <Kbd>⎋</Kbd>
          {t("hintClose")}
        </span>
      </div>
    </CommandDialog>
  );
}

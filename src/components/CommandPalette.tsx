import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Desktop,
  DownloadSimple,
  GearSix,
  Globe,
  MagnifyingGlass,
  Moon,
  Palette,
  Plus,
  SidebarSimple,
  Sun,
  TerminalWindow,
  TextAa,
  X,
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
  type Workspace,
  type WorkspaceId,
} from "../lib/ipc";
import { type Bindings } from "../lib/keys";

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
  mode: "actions" | "sessions";
  onModeChange: (mode: "actions" | "sessions") => void;
  workspaces: Workspace[];
  activeWorkspace: WorkspaceId | null;
  bindings: Bindings;
  theme: ThemeMode;
  onChangeTheme: (mode: ThemeMode) => void;
  onNewSession: () => void;
  onNewTab: () => void;
  onCloseActive: () => void;
  onOpenSettings: () => void;
  onTogglePanel: () => void;
  onGoToWorkspace: (id: WorkspaceId) => void;
}

export function CommandPalette({
  open,
  onOpenChange,
  mode,
  onModeChange,
  workspaces,
  activeWorkspace,
  bindings,
  theme,
  onChangeTheme,
  onNewSession,
  onNewTab,
  onCloseActive,
  onOpenSettings,
  onTogglePanel,
  onGoToWorkspace,
}: Props) {
  const { t, i18n } = useTranslation();
  const [selectableThemes, setSelectableThemes] = useState<Theme[]>([]);

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

  const run = (fn: () => void) => () => {
    onOpenChange(false);
    fn();
  };

  const importTheme = async () => {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: "Tema TYBA (JSON)", extensions: ["json"] }],
    });
    if (typeof path !== "string") return;
    try {
      const imported = await importThemeCmd(path);
      await applyTheme(imported);
    } catch (error) {
      window.alert(t("themeImportFailed", { error: String(error) }));
    }
  };

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("commandPalette")}
      description={t("searchCommand")}
      showCloseButton={false}
      className="top-28 max-w-[560px] translate-y-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface shadow-2xl"
    >
      <div className="flex items-center gap-1 border-b border-tyba-border px-2 py-1.5">
        {(["actions", "sessions"] as const).map((m) => (
          <button
            key={m}
            onClick={() => onModeChange(m)}
            className={`flex items-center gap-1.5 rounded-[4px] px-2 py-1 text-[11px] transition-colors ${
              mode === m
                ? "bg-white/[.06] text-tyba-text"
                : "text-tyba-text-faint hover:text-tyba-text-muted"
            }`}
          >
            {m === "actions" ? (
              <MagnifyingGlass size={12} />
            ) : (
              <TerminalWindow size={12} />
            )}
            {m === "actions" ? t("actions") : t("sessions")}
            <Shortcut
              combo={
                m === "actions"
                  ? bindings.paletteActions
                  : bindings.paletteSessions
              }
              className="ml-1"
            />
          </button>
        ))}
      </div>
      <CommandInput
        placeholder={mode === "sessions" ? t("searchSessions") : t("searchCommand")}
      />
      <CommandList>
        <CommandEmpty>{t("noResults")}</CommandEmpty>

        {mode === "actions" && (
        <CommandGroup heading={t("actions")}>
          <CommandItem onSelect={run(onNewSession)}>
            <Plus size={15} />
            {t("newSession")}
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
          <CommandItem onSelect={run(onOpenSettings)}>
            <GearSix size={15} />
            {t("settings")}
          </CommandItem>
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

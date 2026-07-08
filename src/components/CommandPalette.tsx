import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Desktop,
  DownloadSimple,
  GearSix,
  Globe,
  Moon,
  Palette,
  Plus,
  SidebarSimple,
  Sun,
  TerminalWindow,
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
  CommandShortcut,
} from "@/components/ui/command";
import { LANGUAGES, setLanguage } from "../i18n";
import { applyTheme, THEMES, type Theme, type ThemeMode } from "../theme";
import {
  importThemeCmd,
  listThemes,
  type Workspace,
  type WorkspaceId,
} from "../lib/ipc";

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
  workspaces: Workspace[];
  activeWorkspace: WorkspaceId | null;
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
  workspaces,
  activeWorkspace,
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
  const [customThemes, setCustomThemes] = useState<Theme[]>([]);

  useEffect(() => {
    if (!open) return;
    void listThemes()
      .then((all) => setCustomThemes(all.filter((item) => !item.builtin)))
      .catch(() => setCustomThemes([]));
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
      <CommandInput placeholder={t("searchCommand")} />
      <CommandList>
        <CommandEmpty>{t("noResults")}</CommandEmpty>

        <CommandGroup heading={t("actions")}>
          <CommandItem onSelect={run(onNewSession)}>
            <Plus size={15} />
            {t("newSession")}
          </CommandItem>
          <CommandItem onSelect={run(onNewTab)}>
            <Plus size={15} />
            {t("newTab")}
            <CommandShortcut>⌘T</CommandShortcut>
          </CommandItem>
          {activeWorkspace && (
            <CommandItem onSelect={run(onCloseActive)}>
              <X size={15} />
              {t("closePane")}
              <CommandShortcut>⌘W</CommandShortcut>
            </CommandItem>
          )}
          <CommandItem onSelect={run(onTogglePanel)}>
            <SidebarSimple size={15} />
            {t("togglePanel")}
            <CommandShortcut>⌘B</CommandShortcut>
          </CommandItem>
          <CommandItem onSelect={run(onOpenSettings)}>
            <GearSix size={15} />
            {t("settings")}
          </CommandItem>
        </CommandGroup>

        {workspaces.length > 0 && (
          <>
            <CommandSeparator />
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

        <CommandSeparator />
        <CommandGroup heading={t("theme")}>
          {THEMES.filter((m) => m !== theme).map((mode) => {
            const Icon = THEME_ICONS[mode];
            return (
              <CommandItem key={mode} onSelect={run(() => onChangeTheme(mode))}>
                <Icon size={15} />
                {t("switchThemeTo", { theme: t(THEME_LABEL_KEYS[mode]) })}
              </CommandItem>
            );
          })}
          {customThemes.map((custom) => (
            <CommandItem
              key={custom.id}
              value={`theme ${custom.name}`}
              onSelect={run(() => void applyTheme(custom))}
            >
              <Palette size={15} />
              {t("useTheme", { name: custom.name })}
            </CommandItem>
          ))}
          <CommandItem onSelect={run(() => void importTheme())}>
            <DownloadSimple size={15} />
            {t("importTheme")}
          </CommandItem>
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
      </CommandList>
      <div className="flex items-center gap-4 border-t border-tyba-border px-3 py-1.5 font-mono text-[10px] text-tyba-text-faint">
        <span>↑↓ {t("hintNavigate")}</span>
        <span>↵ {t("hintRun")}</span>
        <span className="ml-auto">esc {t("hintClose")}</span>
      </div>
    </CommandDialog>
  );
}

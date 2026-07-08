// Paleta de comandos (⌘K): navegação por teclado como cidadã de
// primeira classe. Ações do shell + salto direto pra qualquer sessão.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Desktop,
  DownloadSimple,
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
  type Session,
  type SessionId,
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
  sessions: Session[];
  activeId: SessionId | null;
  theme: ThemeMode;
  onChangeTheme: (mode: ThemeMode) => void;
  onNewSession: () => void;
  onCloseActive: () => void;
  onKillActive: () => void;
  onTogglePanel: () => void;
  onGoToSession: (id: SessionId) => void;
}

export function CommandPalette({
  open,
  onOpenChange,
  sessions,
  activeId,
  theme,
  onChangeTheme,
  onNewSession,
  onCloseActive,
  onKillActive,
  onTogglePanel,
  onGoToSession,
}: Props) {
  const { t, i18n } = useTranslation();
  const [customThemes, setCustomThemes] = useState<Theme[]>([]);

  useEffect(() => {
    if (!open) return;
    void listThemes()
      .then((all) => setCustomThemes(all.filter((theme) => !theme.builtin)))
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
    >
      <CommandInput placeholder={t("searchCommand")} />
      <CommandList>
        <CommandEmpty>{t("noResults")}</CommandEmpty>

        <CommandGroup heading={t("actions")}>
          <CommandItem onSelect={run(onNewSession)}>
            <Plus size={15} />
            {t("newSession")}
            <CommandShortcut>⌘T</CommandShortcut>
          </CommandItem>
          {activeId && (
            <CommandItem onSelect={run(onCloseActive)}>
              <X size={15} />
              {t("closePane")}
              <CommandShortcut>⌘W</CommandShortcut>
            </CommandItem>
          )}
          {activeId && (
            <CommandItem onSelect={run(onKillActive)}>
              <X size={15} weight="bold" />
              {t("killSession")}
            </CommandItem>
          )}
          <CommandItem onSelect={run(onTogglePanel)}>
            <SidebarSimple size={15} />
            {t("togglePanel")}
            <CommandShortcut>⌘B</CommandShortcut>
          </CommandItem>
        </CommandGroup>

        {sessions.length > 0 && (
          <>
            <CommandSeparator />
            <CommandGroup heading={t("sessions")}>
              {sessions.map((s) => (
                <CommandItem
                  key={s.id}
                  value={`${s.title} ${s.id}`}
                  onSelect={run(() => onGoToSession(s.id))}
                >
                  <TerminalWindow
                    size={15}
                    className={
                      s.id === activeId ? "text-tyba-green" : undefined
                    }
                  />
                  <span className="truncate">{s.title}</span>
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
              <CommandItem
                key={mode}
                onSelect={run(() => onChangeTheme(mode))}
              >
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
    </CommandDialog>
  );
}

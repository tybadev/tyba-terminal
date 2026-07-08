import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Check,
  DownloadSimple,
  Palette,
  SlidersHorizontal,
  User,
  X,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { LANGUAGES, setLanguage, type LanguageCode } from "../i18n";
import {
  applyTheme,
  getThemeMode,
  onThemeModeChange,
  setThemeMode,
  THEMES,
  type Theme,
  type ThemeMode,
} from "../theme";
import {
  getThemeState,
  importThemeCmd,
  listThemes,
  onThemeChanged,
  type ThemeState,
} from "../lib/ipc";

export type SidebarTogglePref = "hidden" | "rail";

type Section = "account" | "themes" | "preferences";

interface Props {
  onClose: () => void;
  togglePref: SidebarTogglePref;
  onTogglePrefChange: (value: SidebarTogglePref) => void;
}

const THEME_MODE_KEYS: Record<ThemeMode, string> = {
  dark: "themeDark",
  light: "themeLight",
  system: "themeSystem",
};

function NavItem({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`relative flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] transition-colors ${
        active
          ? "text-tyba-text"
          : "text-tyba-text-faint hover:bg-white/[.03] hover:text-tyba-text-muted"
      }`}
    >
      {active && (
        <span
          className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full"
          style={{ background: "var(--tyba-gradient-soft)" }}
        />
      )}
      {icon}
      {label}
    </button>
  );
}

function Choice({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex h-8 items-center gap-2 rounded-[4px] border px-3 text-[13px] transition-colors ${
        active
          ? "border-tyba-border-strong bg-white/[.04] text-tyba-text"
          : "border-tyba-border text-tyba-text-muted hover:text-tyba-text"
      }`}
    >
      {active && <Check size={12} weight="bold" className="text-tyba-green" />}
      {label}
    </button>
  );
}

export function SettingsView({ onClose, togglePref, onTogglePrefChange }: Props) {
  const { t, i18n } = useTranslation();
  const [section, setSection] = useState<Section>("themes");
  const [mode, setMode] = useState<ThemeMode>(getThemeMode);
  const [themes, setThemes] = useState<Theme[]>([]);
  const [themeState, setThemeState] = useState<ThemeState | null>(null);

  const refresh = useCallback(() => {
    void listThemes().then(setThemes).catch(() => {});
    void getThemeState().then(setThemeState).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const offMode = onThemeModeChange(setMode);
    let unlisten: (() => void) | null = null;
    void onThemeChanged((state) => setThemeState(state)).then((un) => {
      unlisten = un;
    });
    return () => {
      offMode();
      unlisten?.();
    };
  }, [refresh]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const importTheme = async () => {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: "Tema TYBA (JSON)", extensions: ["json"] }],
    });
    if (typeof path !== "string") return;
    try {
      await importThemeCmd(path);
      refresh();
    } catch (error) {
      window.alert(t("themeImportFailed", { error: String(error) }));
    }
  };

  const slotOf = (theme: Theme) =>
    theme.base === "dark" ? themeState?.dark.id : themeState?.light.id;

  return (
    <div className="flex min-h-0 flex-1">
      <aside className="flex w-48 shrink-0 flex-col gap-px px-2 pt-3">
        <span className="tyba-label px-2.5 pb-2">{t("settings")}</span>
        <NavItem
          active={section === "account"}
          icon={<User size={15} />}
          label={t("settingsAccount")}
          onClick={() => setSection("account")}
        />
        <NavItem
          active={section === "themes"}
          icon={<Palette size={15} />}
          label={t("settingsThemes")}
          onClick={() => setSection("themes")}
        />
        <NavItem
          active={section === "preferences"}
          icon={<SlidersHorizontal size={15} />}
          label={t("settingsPreferences")}
          onClick={() => setSection("preferences")}
        />
      </aside>

      <div className="relative min-w-0 flex-1 overflow-y-auto px-8 pt-4 pb-8">
        <button
          onClick={onClose}
          aria-label={t("hintClose")}
          className="absolute right-4 top-4 rounded-[4px] p-1 text-tyba-text-faint transition-colors hover:bg-white/[.04] hover:text-tyba-text"
        >
          <X size={14} weight="bold" />
        </button>

        {section === "account" && (
          <section className="max-w-lg">
            <h2 className="pb-1 text-sm font-medium">{t("settingsAccount")}</h2>
            <p className="pb-6 text-[12px] text-tyba-text-faint">
              {t("localAccount")}
            </p>
            <div className="flex items-center gap-3 rounded-[6px] border border-tyba-border px-4 py-3">
              <span
                className="rounded-full p-px"
                style={{ background: "var(--tyba-gradient)" }}
              >
                <span className="flex size-8 items-center justify-center rounded-full bg-tyba-raised text-tyba-text-muted">
                  <User size={15} weight="bold" />
                </span>
              </span>
              <div className="min-w-0">
                <p className="text-[13px]">{t("localAccount")}</p>
                <p className="text-[11px] text-tyba-text-faint">
                  {t("accountHint")}
                </p>
              </div>
            </div>
          </section>
        )}

        {section === "themes" && (
          <section className="max-w-lg">
            <h2 className="pb-1 text-sm font-medium">{t("settingsThemes")}</h2>
            <p className="pb-5 text-[12px] text-tyba-text-faint">
              {t("themesHint")}
            </p>

            <span className="tyba-label">{t("theme")}</span>
            <div className="flex gap-2 pt-2 pb-6">
              {THEMES.map((m) => (
                <Choice
                  key={m}
                  active={mode === m}
                  label={t(THEME_MODE_KEYS[m])}
                  onClick={() => setThemeMode(m)}
                />
              ))}
            </div>

            <div className="flex items-center justify-between pb-2">
              <span className="tyba-label">{t("settingsThemes")}</span>
              <Button
                variant="ghost"
                size="xs"
                onClick={() => void importTheme()}
                className="text-tyba-text-muted hover:text-tyba-text"
              >
                <DownloadSimple size={13} />
                {t("importTheme")}
              </Button>
            </div>
            <div className="flex flex-col gap-px">
              {themes.map((theme) => {
                const inUse = slotOf(theme) === theme.id;
                return (
                  <div
                    key={theme.id}
                    className="flex h-9 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] hover:bg-white/[.03]"
                  >
                    <span
                      className="size-3 shrink-0 rounded-full border border-tyba-border-strong"
                      style={{ background: theme.terminal.background }}
                    />
                    <span className="min-w-0 flex-1 truncate">{theme.name}</span>
                    <span className="font-mono text-[10px] text-tyba-text-faint">
                      {theme.base}
                    </span>
                    {inUse ? (
                      <span className="flex items-center gap-1 font-mono text-[10px] text-tyba-green">
                        <Check size={11} weight="bold" />
                        {t("themeInUse")}
                      </span>
                    ) : (
                      <button
                        onClick={() => void applyTheme(theme)}
                        className="rounded-[4px] px-2 py-0.5 text-[11px] text-tyba-text-muted transition-colors hover:bg-white/[.05] hover:text-tyba-text"
                      >
                        {t("useThemeShort")}
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          </section>
        )}

        {section === "preferences" && (
          <section className="max-w-lg">
            <h2 className="pb-1 text-sm font-medium">
              {t("settingsPreferences")}
            </h2>
            <p className="pb-5 text-[12px] text-tyba-text-faint">
              {t("preferencesHint")}
            </p>

            <span className="tyba-label">{t("sidebarToggleBehavior")}</span>
            <div className="flex gap-2 pt-2 pb-6">
              <Choice
                active={togglePref === "hidden"}
                label={t("collapseAll")}
                onClick={() => onTogglePrefChange("hidden")}
              />
              <Choice
                active={togglePref === "rail"}
                label={t("collapseRail")}
                onClick={() => onTogglePrefChange("rail")}
              />
            </div>

            <span className="tyba-label">{t("language")}</span>
            <div className="flex gap-2 pt-2">
              {LANGUAGES.map((lang) => (
                <Choice
                  key={lang.code}
                  active={i18n.language === lang.code}
                  label={lang.label}
                  onClick={() => setLanguage(lang.code as LanguageCode)}
                />
              ))}
            </div>
          </section>
        )}
      </div>
    </div>
  );
}

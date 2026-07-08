import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Check,
  Code,
  DownloadSimple,
  FolderOpen,
  Keyboard,
  Palette,
  SlidersHorizontal,
  User,
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
  getPref,
  getThemeState,
  importThemeCmd,
  listThemes,
  onThemeChanged,
  setPref,
  type ThemeState,
} from "../lib/ipc";
import {
  captureState,
  comboOf,
  formatCombo,
  KEY_ACTIONS,
  type Bindings,
  type KeyAction,
} from "../lib/keys";
import { FONT_SIZE_EVENT, setDefaultFontSize } from "./TerminalView";

export type SidebarTogglePref = "hidden" | "rail";
export type DetailsPref = "on" | "off";

type Section =
  | "account"
  | "general"
  | "code"
  | "themes"
  | "shortcuts"
  | "preferences";

interface Props {
  onClose: () => void;
  togglePref: SidebarTogglePref;
  onTogglePrefChange: (value: SidebarTogglePref) => void;
  detailsPref: DetailsPref;
  onDetailsPrefChange: (value: DetailsPref) => void;
  bindings: Bindings;
  onBindingsChange: (value: Bindings) => void;
  accountName: string;
  onAccountNameChange: (value: string) => void;
}

const THEME_MODE_KEYS: Record<ThemeMode, string> = {
  dark: "themeDark",
  light: "themeLight",
  system: "themeSystem",
};

const ACTION_LABEL_KEYS: Record<KeyAction, string> = {
  palette: "commandPalette",
  panel: "togglePanel",
  newTab: "newTab",
  closePane: "closePane",
  openFolder: "openProjectFolder",
};

const FONT_SIZES = [11, 12, 13, 14, 15, 16];

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

function SectionHeader({ title, hint }: { title: string; hint: string }) {
  return (
    <>
      <h2 className="pb-1 text-sm font-medium">{title}</h2>
      <p className="pb-5 text-[12px] text-tyba-text-faint">{hint}</p>
    </>
  );
}

function TextField({
  value,
  placeholder,
  onCommit,
}: {
  value: string;
  placeholder: string;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <input
      value={draft}
      placeholder={placeholder}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => onCommit(draft.trim())}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
      }}
      className="h-8 w-full rounded-[4px] border border-tyba-border bg-white/[.02] px-2.5 text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-border-strong"
    />
  );
}

function ShortcutRow({
  action,
  combo,
  onRebind,
}: {
  action: KeyAction;
  combo: string;
  onRebind: (combo: string) => void;
}) {
  const { t } = useTranslation();
  const [listening, setListening] = useState(false);

  useEffect(() => {
    if (!listening) return;
    captureState.active = true;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setListening(false);
        return;
      }
      const next = comboOf(e);
      if (next) {
        onRebind(next);
        setListening(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      captureState.active = false;
      window.removeEventListener("keydown", onKey, true);
    };
  }, [listening, onRebind]);

  return (
    <div className="flex h-9 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] hover:bg-white/[.03]">
      <span className="min-w-0 flex-1 truncate">
        {t(ACTION_LABEL_KEYS[action])}
      </span>
      <button
        onClick={() => setListening(true)}
        className={`rounded-[4px] border px-2 py-0.5 font-mono text-[11px] transition-colors ${
          listening
            ? "border-tyba-green text-tyba-green"
            : "border-tyba-border-strong text-tyba-text-muted hover:text-tyba-text"
        }`}
      >
        {listening ? t("pressKeys") : formatCombo(combo)}
      </button>
    </div>
  );
}

export function SettingsView({
  onClose,
  togglePref,
  onTogglePrefChange,
  detailsPref,
  onDetailsPrefChange,
  bindings,
  onBindingsChange,
  accountName,
  onAccountNameChange,
}: Props) {
  const { t, i18n } = useTranslation();
  const [section, setSection] = useState<Section>("general");
  const [mode, setMode] = useState<ThemeMode>(getThemeMode);
  const [themes, setThemes] = useState<Theme[]>([]);
  const [themeState, setThemeState] = useState<ThemeState | null>(null);
  const [defaultDir, setDefaultDir] = useState("");
  const [fontSize, setFontSize] = useState(13);

  const refresh = useCallback(() => {
    void listThemes().then(setThemes).catch(() => {});
    void getThemeState().then(setThemeState).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    void getPref("pref.default_session_dir")
      .then((v) => setDefaultDir(v ?? ""))
      .catch(() => {});
    void getPref("pref.code.font_size")
      .then((v) => {
        const n = Number(v);
        if (n >= 10 && n <= 20) setFontSize(n);
      })
      .catch(() => {});
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

  const commitDefaultDir = useCallback((dir: string) => {
    setDefaultDir(dir);
    void setPref("pref.default_session_dir", dir).catch(() => {});
  }, []);

  const chooseDefaultDir = async () => {
    const dir = await openFileDialog({ directory: true, multiple: false });
    if (typeof dir === "string") commitDefaultDir(dir);
  };

  const changeFontSize = (size: number) => {
    setFontSize(size);
    setDefaultFontSize(size);
    void setPref("pref.code.font_size", String(size)).catch(() => {});
    window.dispatchEvent(new CustomEvent(FONT_SIZE_EVENT, { detail: size }));
  };

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
        <button
          onClick={onClose}
          className="mb-2 flex h-8 items-center gap-2 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-white/[.03] hover:text-tyba-text"
        >
          <ArrowLeft size={14} />
          <span className="flex-1 text-left">{t("back")}</span>
          <kbd className="rounded-[4px] border border-tyba-border-strong bg-tyba-raised px-1 py-px font-mono text-[10px] text-tyba-text-muted">
            {formatCombo(bindings.closePane)}
          </kbd>
        </button>
        <NavItem
          active={section === "general"}
          icon={<SlidersHorizontal size={15} />}
          label={t("settingsGeneral")}
          onClick={() => setSection("general")}
        />
        <NavItem
          active={section === "account"}
          icon={<User size={15} />}
          label={t("settingsAccount")}
          onClick={() => setSection("account")}
        />
        <NavItem
          active={section === "code"}
          icon={<Code size={15} />}
          label={t("settingsCode")}
          onClick={() => setSection("code")}
        />
        <NavItem
          active={section === "themes"}
          icon={<Palette size={15} />}
          label={t("settingsThemes")}
          onClick={() => setSection("themes")}
        />
        <NavItem
          active={section === "shortcuts"}
          icon={<Keyboard size={15} />}
          label={t("settingsShortcuts")}
          onClick={() => setSection("shortcuts")}
        />
        <NavItem
          active={section === "preferences"}
          icon={<SlidersHorizontal size={15} />}
          label={t("settingsPreferences")}
          onClick={() => setSection("preferences")}
        />
      </aside>

      <div className="min-w-0 flex-1 overflow-y-auto px-8 pt-4 pb-8">
        {section === "general" && (
          <section className="max-w-lg">
            <SectionHeader
              title={t("settingsGeneral")}
              hint={t("generalHint")}
            />
            <span className="tyba-label">{t("defaultSessionDir")}</span>
            <div className="flex gap-2 pt-2">
              <TextField
                value={defaultDir}
                placeholder="~"
                onCommit={commitDefaultDir}
              />
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void chooseDefaultDir()}
                className="shrink-0 text-tyba-text-muted hover:text-tyba-text"
              >
                <FolderOpen size={14} />
                {t("chooseFolder")}
              </Button>
            </div>
            <p className="pt-2 text-[11px] text-tyba-text-faint">
              {t("defaultSessionDirHint")}
            </p>
          </section>
        )}

        {section === "account" && (
          <section className="max-w-lg">
            <SectionHeader
              title={t("settingsAccount")}
              hint={t("accountHint")}
            />
            <div className="flex items-center gap-3 rounded-[6px] border border-tyba-border px-4 py-3">
              <span
                className="rounded-full p-px"
                style={{ background: "var(--tyba-gradient)" }}
              >
                <span className="flex size-8 items-center justify-center rounded-full bg-tyba-raised text-tyba-text-muted">
                  <User size={15} weight="bold" />
                </span>
              </span>
              <div className="min-w-0 flex-1">
                <p className="pb-1 text-[13px]">{t("localAccount")}</p>
                <TextField
                  value={accountName}
                  placeholder={t("accountNamePlaceholder")}
                  onCommit={onAccountNameChange}
                />
              </div>
            </div>
          </section>
        )}

        {section === "code" && (
          <section className="max-w-lg">
            <SectionHeader title={t("settingsCode")} hint={t("codeHint")} />
            <span className="tyba-label">{t("terminalFontSize")}</span>
            <div className="flex gap-2 pt-2">
              {FONT_SIZES.map((size) => (
                <Choice
                  key={size}
                  active={fontSize === size}
                  label={`${size}px`}
                  onClick={() => changeFontSize(size)}
                />
              ))}
            </div>
          </section>
        )}

        {section === "themes" && (
          <section className="max-w-lg">
            <SectionHeader
              title={t("settingsThemes")}
              hint={t("themesHint")}
            />
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
                    <span className="min-w-0 flex-1 truncate">
                      {theme.name}
                    </span>
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

        {section === "shortcuts" && (
          <section className="max-w-lg">
            <SectionHeader
              title={t("settingsShortcuts")}
              hint={t("shortcutsHint")}
            />
            <div className="flex flex-col gap-px">
              {KEY_ACTIONS.map((action) => (
                <ShortcutRow
                  key={action}
                  action={action}
                  combo={bindings[action]}
                  onRebind={(combo) =>
                    onBindingsChange({ ...bindings, [action]: combo })
                  }
                />
              ))}
            </div>
          </section>
        )}

        {section === "preferences" && (
          <section className="max-w-lg">
            <SectionHeader
              title={t("settingsPreferences")}
              hint={t("preferencesHint")}
            />
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

            <span className="tyba-label">{t("sidebarDetails")}</span>
            <div className="flex gap-2 pt-2 pb-6">
              <Choice
                active={detailsPref === "on"}
                label={t("detailsShow")}
                onClick={() => onDetailsPrefChange("on")}
              />
              <Choice
                active={detailsPref === "off"}
                label={t("detailsHide")}
                onClick={() => onDetailsPrefChange("off")}
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

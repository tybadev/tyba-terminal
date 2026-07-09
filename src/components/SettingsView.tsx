import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Code,
  DownloadSimple,
  FolderOpen,
  Keyboard,
  Palette,
  SlidersHorizontal,
  TerminalWindow,
  User,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { DockerIcon } from "./icons/DockerIcon";
import { LANGUAGES, setLanguage, type LanguageCode } from "../i18n";
import {
  getUiFont,
  onUiFontChange,
  setUiFont,
  UI_FONT_LABELS,
  UI_FONTS,
  type UiFont,
} from "../font";
import {
  applyTheme,
  getEffectiveBase,
  getThemeMode,
  onEffectiveBaseChange,
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
  actionsByCategory,
  ACTION_LABEL_KEYS,
  captureState,
  comboOf,
  KEY_CATEGORY_LABEL_KEYS,
  type Bindings,
  type KeyAction,
} from "../lib/keys";
import { Shortcut } from "@/components/ui/kbd";
import { FONT_SIZE_EVENT, setDefaultFontSize } from "./TerminalView";

export type SidebarTogglePref = "hidden" | "rail";
export type DetailsPref = "on" | "off";

type Section =
  | "general"
  | "appearance"
  | "code"
  | "shortcuts"
  | "preferences";

interface Props {
  togglePref: SidebarTogglePref;
  onTogglePrefChange: (value: SidebarTogglePref) => void;
  detailsPref: DetailsPref;
  onDetailsPrefChange: (value: DetailsPref) => void;
  bindings: Bindings;
  onBindingsChange: (value: Bindings) => void;
  accountName: string;
  onAccountNameChange: (value: string) => void;
  showContainers: boolean;
  onShowContainersChange: (value: boolean) => void;
  showGitStatus: boolean;
  onShowGitStatusChange: (value: boolean) => void;
  shellIntegration: boolean;
  onShellIntegrationChange: (value: boolean) => void;
}

const THEME_MODE_KEYS: Record<ThemeMode, string> = {
  dark: "themeDark",
  light: "themeLight",
  system: "themeSystem",
};

const FONT_SIZES = [11, 12, 13, 14, 15, 16];

const THEME_EXAMPLE = `{
  "name": "Meu Tema",
  "base": "dark",
  "ui": {
    "bg": "#282a36",
    "surface": "#313342",
    "text": "#f8f8f2",
    "primary": "#50fa7b",
    "violet": "#bd93f9"
  },
  "terminal": {
    "background": "#282a36",
    "foreground": "#f8f8f2",
    "cursor": "#f8f8f2",
    "selectionBackground": "#44475a80",
    "ansi": ["#21222c", "#ff5555", "#50fa7b", "#f1fa8c",
             "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
             "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5",
             "#d6acff", "#ff92df", "#a4ffff", "#ffffff"]
  }
}`;

function copyThemeExample() {
  void navigator.clipboard?.writeText(THEME_EXAMPLE).catch(() => {});
}

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
          ? "bg-white/[.05] text-tyba-text"
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
        className={`flex h-7 items-center gap-1 rounded-[4px] border px-2 text-[11px] transition-colors ${
          listening
            ? "border-tyba-green font-mono text-tyba-green"
            : "border-transparent hover:bg-white/[.04]"
        }`}
      >
        {listening ? t("pressKeys") : <Shortcut combo={combo} />}
      </button>
    </div>
  );
}

export function SettingsView({
  togglePref,
  onTogglePrefChange,
  detailsPref,
  onDetailsPrefChange,
  bindings,
  onBindingsChange,
  accountName,
  onAccountNameChange,
  showContainers,
  onShowContainersChange,
  showGitStatus,
  onShowGitStatusChange,
  shellIntegration,
  onShellIntegrationChange,
}: Props) {
  const { t, i18n } = useTranslation();
  const [section, setSection] = useState<Section>("general");
  const [mode, setMode] = useState<ThemeMode>(getThemeMode);
  const [themes, setThemes] = useState<Theme[]>([]);
  const [themeState, setThemeState] = useState<ThemeState | null>(null);
  const [defaultDir, setDefaultDir] = useState("");
  const [fontSize, setFontSize] = useState(13);
  const [uiFont, setUiFontState] = useState<UiFont>(getUiFont);
  const [effBase, setEffBase] = useState(getEffectiveBase);

  useEffect(() => onUiFontChange(setUiFontState), []);
  useEffect(
    () => onEffectiveBaseChange(() => setEffBase(getEffectiveBase())),
    [],
  );

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

  const effectiveThemeId = themeState ? themeState[effBase].id : null;

  return (
    <div className="flex min-h-0 flex-1">
      <aside className="flex w-48 shrink-0 flex-col gap-px border-x border-tyba-border bg-tyba-surface px-2 pt-3">
        <NavItem
          active={section === "general"}
          icon={<User size={15} />}
          label={t("settingsGeneral")}
          onClick={() => setSection("general")}
        />
        <NavItem
          active={section === "appearance"}
          icon={<Palette size={15} />}
          label={t("settingsAppearance")}
          onClick={() => setSection("appearance")}
        />
        <NavItem
          active={section === "code"}
          icon={<Code size={15} />}
          label={t("settingsCode")}
          onClick={() => setSection("code")}
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
          <section className="mx-auto w-full max-w-lg">
            <SectionHeader
              title={t("settingsGeneral")}
              hint={t("generalHint")}
            />
            <span className="tyba-label">{t("account")}</span>
            <div className="mt-2 mb-6 flex items-center gap-3 rounded-[6px] border border-tyba-border px-4 py-3">
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

        {section === "code" && (
          <section className="mx-auto w-full max-w-lg">
            <SectionHeader title={t("settingsCode")} hint={t("codeHint")} />
            <span className="tyba-label">{t("integrations")}</span>
            <div className="mt-2 flex items-start gap-3 rounded-[6px] border border-tyba-border p-4">
              <span className="mt-0.5 shrink-0 text-tyba-text-muted">
                <DockerIcon size={18} />
              </span>
              <div className="min-w-0 flex-1">
                <p className="text-[13px] text-tyba-text">
                  {t("dockerIntegration")}
                </p>
                <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
                  {t("dockerIntegrationHint")}
                </p>
              </div>
              <Switch
                checked={showContainers}
                onCheckedChange={onShowContainersChange}
                aria-label={t("dockerIntegration")}
                className="mt-0.5"
              />
            </div>

            <div className="mt-2 flex items-start gap-3 rounded-[6px] border border-tyba-border p-4">
              <span className="mt-0.5 shrink-0 text-tyba-text-muted">
                <TerminalWindow size={18} />
              </span>
              <div className="min-w-0 flex-1">
                <p className="text-[13px] text-tyba-text">
                  {t("shellIntegration")}
                </p>
                <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
                  {t("shellIntegrationHint")}
                </p>
              </div>
              <Switch
                checked={shellIntegration}
                onCheckedChange={onShellIntegrationChange}
                aria-label={t("shellIntegration")}
                className="mt-0.5"
              />
            </div>
          </section>
        )}

        {section === "appearance" && (
          <section className="mx-auto w-full max-w-lg">
            <SectionHeader
              title={t("settingsAppearance")}
              hint={t("appearanceHint")}
            />
            <span className="tyba-label">{t("colorMode")}</span>
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

            <span className="tyba-label">{t("fontSection")}</span>
            <div className="grid grid-cols-2 gap-2 pt-2 pb-6">
              <Select
                value={uiFont}
                onChange={(v) => setUiFont(v as UiFont)}
                options={UI_FONTS.map((f) => ({
                  value: f,
                  label: UI_FONT_LABELS[f],
                }))}
              />
              <Select
                value={String(fontSize)}
                onChange={(v) => changeFontSize(Number(v))}
                options={FONT_SIZES.map((s) => ({
                  value: String(s),
                  label: `${s}px`,
                }))}
              />
            </div>

            <div className="flex items-center justify-between pb-1">
              <span className="tyba-label">{t("settingsThemes")}</span>
              <div className="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="xs"
                  onClick={() => copyThemeExample()}
                  className="text-tyba-text-muted hover:text-tyba-text"
                >
                  {t("copyThemeExample")}
                </Button>
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
            </div>
            <p className="pb-3 text-[11px] text-tyba-text-faint">
              {t("themeImportHint")}
            </p>
            <div className="grid grid-cols-2 gap-2">
              {themes.map((theme) => {
                const inUse = theme.id === effectiveThemeId;
                const ansi = theme.terminal.ansi;
                return (
                  <button
                    key={theme.id}
                    onClick={() => void applyTheme(theme)}
                    aria-pressed={inUse}
                    className={`group relative flex flex-col gap-2 overflow-hidden rounded-[6px] border p-2 text-left transition-colors ${
                      inUse
                        ? "border-tyba-green [box-shadow:0_0_0_1px_var(--tyba-green)]"
                        : "border-tyba-border hover:border-tyba-border-strong"
                    }`}
                  >
                    <div
                      className="flex h-11 items-end gap-1 rounded-[4px] p-2"
                      style={{ background: theme.terminal.background }}
                    >
                      {[1, 2, 3, 4, 5, 6].map((i) => (
                        <span
                          key={i}
                          className="h-4 flex-1 rounded-[2px]"
                          style={{ background: ansi[i] }}
                        />
                      ))}
                    </div>
                    <div className="flex items-center gap-1.5">
                      <span className="min-w-0 flex-1 truncate text-[12px] text-tyba-text">
                        {theme.name}
                      </span>
                      {inUse && (
                        <Check
                          size={12}
                          weight="bold"
                          className="shrink-0 text-tyba-green"
                        />
                      )}
                    </div>
                  </button>
                );
              })}
            </div>
          </section>
        )}

        {section === "shortcuts" && (
          <section className="mx-auto w-full max-w-lg">
            <SectionHeader
              title={t("settingsShortcuts")}
              hint={t("shortcutsHint")}
            />
            {actionsByCategory().map(([category, actions]) =>
              actions.length === 0 ? null : (
                <div key={category} className="mb-5 last:mb-0">
                  <span className="tyba-label">
                    {t(KEY_CATEGORY_LABEL_KEYS[category])}
                  </span>
                  <div className="flex flex-col gap-px pt-2">
                    {actions.map((action) => (
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
                </div>
              ),
            )}
          </section>
        )}

        {section === "preferences" && (
          <section className="mx-auto w-full max-w-lg">
            <SectionHeader
              title={t("settingsPreferences")}
              hint={t("preferencesHint")}
            />
            <span className="tyba-label">{t("sidebarToggleBehavior")}</span>
            <div className="pt-2 pb-6">
              <Select
                value={togglePref}
                onChange={(v) => onTogglePrefChange(v as SidebarTogglePref)}
                className="w-56"
                options={[
                  { value: "hidden", label: t("collapseAll") },
                  { value: "rail", label: t("collapseRail") },
                ]}
              />
            </div>

            <label className="flex items-center justify-between gap-4 pb-6">
              <span className="text-[13px] text-tyba-text">
                {t("sidebarDetails")}
              </span>
              <Switch
                checked={detailsPref === "on"}
                onCheckedChange={(c) => onDetailsPrefChange(c ? "on" : "off")}
              />
            </label>

            <label className="flex items-start justify-between gap-4 pb-6">
              <span className="min-w-0">
                <span className="text-[13px] text-tyba-text">
                  {t("gitStatusToggle")}
                </span>
                <span className="block pt-0.5 text-[11px] text-tyba-text-faint">
                  {t("gitStatusHint")}
                </span>
              </span>
              <Switch
                checked={showGitStatus}
                onCheckedChange={onShowGitStatusChange}
                className="mt-0.5"
              />
            </label>

            <span className="tyba-label">{t("language")}</span>
            <div className="pt-2">
              <Select
                value={i18n.language}
                onChange={(v) => setLanguage(v as LanguageCode)}
                className="w-56"
                options={LANGUAGES.map((l) => ({
                  value: l.code,
                  label: l.label,
                }))}
              />
            </div>
          </section>
        )}
      </div>
    </div>
  );
}

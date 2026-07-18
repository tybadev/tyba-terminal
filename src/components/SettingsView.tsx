import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { changelogUrl } from "@/lib/changelog";
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
  listEditors,
  listThemes,
  onThemeChanged,
  setPref,
  type EditorInfo,
  type ThemeState,
  type UpdateStatus,
} from "../lib/ipc";
import {
  actionsByCategory,
  ACTION_LABEL_KEYS,
  captureState,
  comboOf,
  formatCombo,
  KEY_CATEGORY_LABEL_KEYS,
  type Bindings,
  type KeyAction,
} from "../lib/keys";
import { Shortcut } from "@/components/ui/kbd";
import { toastError } from "../lib/toast";
import { FONT_SIZE_EVENT, setDefaultFontSize } from "./TerminalView";
import { ToolbarChipsEditor } from "./ToolbarChipsEditor";
import type { RichInputPref } from "../lib/richInput";
import type { ToolbarPref } from "../lib/repoSnapshots";
import { parseStartupMode } from "../lib/startup";

export type SidebarTogglePref = "hidden" | "rail";
/// Espelha `session::StartupMode` no core: o que fazer com as sessões que não
/// sobreviveram ao fechamento do app.
export type StartupMode = "resume" | "keep_layout" | "fresh";
export type DetailsPref = "on" | "off";

type Section =
  | "general"
  | "appearance"
  | "code"
  | "shortcuts"
  | "preferences";

interface Props {
  version: string;
  update: UpdateStatus | null;
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
  toolbarPref: ToolbarPref;
  onToolbarPrefChange: (value: ToolbarPref) => void;
  worktreeDefault: boolean;
  onWorktreeDefaultChange: (value: boolean) => void;
  onShowGitStatusChange: (value: boolean) => void;
  shellIntegration: boolean;
  onShellIntegrationChange: (value: boolean) => void;
  startup: StartupMode;
  onStartupChange: (value: StartupMode) => void;
  richInputPref: RichInputPref;
  onRichInputPrefChange: (value: RichInputPref) => void;
  richInputRegexInvalid: boolean;
  editor: string;
  onEditorChange: (value: string) => void;
  reviewAgent: string;
  onReviewAgentChange: (value: string) => void;
}

type RichInputToggle = {
  field: keyof Omit<RichInputPref, "version" | "agentRegex">;
  label: string;
  hint?: string;
};

const RICH_INPUT_TOGGLES: RichInputToggle[] = [
  { field: "autoShow", label: "richInputAutoShow", hint: "richInputHint" },
  { field: "autoOpenOnStart", label: "richInputAutoOpen" },
  { field: "autoDismiss", label: "richInputAutoDismiss" },
  {
    field: "submitWithCtrlEnter",
    label: "richInputCtrlEnter",
    hint: "richInputCtrlEnterHint",
  },
  { field: "warnOnSensitivePrompt", label: "richInputWarnSensitive" },
  {
    field: "showOnMatch",
    label: "richInputShowOnMatch",
    hint: "richInputShowOnMatchHint",
  },
];

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
          ? "bg-tyba-text/[.05] text-tyba-text"
          : "text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text-muted"
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
          ? "border-tyba-border-strong bg-tyba-text/[.04] text-tyba-text"
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

function SettingRow({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 px-4 py-3">
      <div className="min-w-0">
        <p className="text-[13px] text-tyba-text">{label}</p>
        {hint && (
          <p className="break-words pt-0.5 text-[11px] leading-relaxed text-tyba-text-faint">
            {hint}
          </p>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
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
      className="h-8 w-full rounded-[4px] border border-tyba-border bg-tyba-text/[.02] px-2.5 text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-border-strong"
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
    <div className="flex h-9 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] hover:bg-tyba-text/[.03]">
      <span className="min-w-0 flex-1 truncate">
        {t(ACTION_LABEL_KEYS[action])}
      </span>
      <button
        onClick={() => setListening(true)}
        className={`flex h-7 items-center gap-1 rounded-[4px] border px-2 text-[11px] transition-colors ${
          listening
            ? "border-tyba-green font-mono text-tyba-green"
            : "border-transparent hover:bg-tyba-text/[.04]"
        }`}
      >
        {listening ? t("pressKeys") : <Shortcut combo={combo} />}
      </button>
    </div>
  );
}

export function SettingsView({
  version,
  update,
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
  toolbarPref,
  onToolbarPrefChange,
  worktreeDefault,
  onWorktreeDefaultChange,
  onShowGitStatusChange,
  shellIntegration,
  onShellIntegrationChange,
  startup,
  onStartupChange,
  richInputPref,
  onRichInputPrefChange,
  richInputRegexInvalid,
  editor,
  onEditorChange,
  reviewAgent,
  onReviewAgentChange,
}: Props) {
  const { t, i18n } = useTranslation();
  const [section, setSection] = useState<Section>("general");
  const [mode, setMode] = useState<ThemeMode>(getThemeMode);
  const [themes, setThemes] = useState<Theme[]>([]);
  const [editors, setEditors] = useState<EditorInfo[]>([]);
  useEffect(() => {
    void listEditors()
      .then(setEditors)
      .catch(() => setEditors([]));
  }, []);
  const editorPath = editors.find((e) => e.id === editor)?.path ?? null;

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
      filters: [{ name: "Tema Tyba (JSON)", extensions: ["json"] }],
    });
    if (typeof path !== "string") return;
    try {
      await importThemeCmd(path);
      refresh();
    } catch (error) {
      toastError(t("themeImportFailedTitle"), error);
    }
  };

  const effectiveThemeId = themeState ? themeState[effBase].id : null;

  return (
    <div className="flex min-h-0 flex-1">
      <aside className="tyba-divide-r flex w-48 shrink-0 flex-col gap-px px-2 pt-3">
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
            <span className="tyba-label">{t("version")}</span>
            <div className="mt-2 mb-6 flex items-center gap-3 rounded-[6px] border border-tyba-border px-4 py-3">
              <div className="min-w-0 flex-1">
                <p className="font-mono text-[13px] text-tyba-text">
                  {version || "—"}
                </p>
                <p className="pt-0.5 text-[11px] text-tyba-text-faint">
                  {update
                    ? t("updateAvailable", { version: update.info.version })
                    : t("updateUpToDate")}
                </p>
              </div>
              {update && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 shrink-0 gap-1.5 px-2.5 text-[11px] text-tyba-violet"
                  onClick={() =>
                    void openUrl(changelogUrl(i18n.language)).catch(() => {})
                  }
                >
                  <DownloadSimple size={13} />
                  {t("updateOpenChangelog")}
                </Button>
              )}
            </div>
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

            <span className="tyba-label mt-6 block">{t("defaultEditor")}</span>
            <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
              {t("defaultEditorHint")}
            </p>
            <div className="mt-2 divide-y divide-tyba-border overflow-hidden rounded-[6px] border border-tyba-border">
              <SettingRow
                label={t("defaultEditor")}
                hint={editorPath ?? t("defaultEditorSystemHint")}
              >
                <Select
                  value={editor}
                  onChange={onEditorChange}
                  className="w-56"
                  options={[
                    { value: "", label: t("defaultEditorSystem") },
                    ...editors.map((e) => ({ value: e.id, label: e.name })),
                  ]}
                />
              </SettingRow>
            </div>

            <span className="tyba-label mt-6 block">{t("startup")}</span>
            <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
              {t("startupHint")}
            </p>
            <div className="mt-2 divide-y divide-tyba-border overflow-hidden rounded-[6px] border border-tyba-border">
              <SettingRow label={t("startup")}>
                <Select
                  value={startup}
                  onChange={(value) => onStartupChange(parseStartupMode(value))}
                  className="w-56"
                  options={[
                    { value: "resume", label: t("startupResume") },
                    { value: "keep_layout", label: t("startupKeepLayout") },
                    { value: "fresh", label: t("startupFresh") },
                  ]}
                />
              </SettingRow>
            </div>

            <span className="tyba-label mt-6 block">{t("reviewAgent")}</span>
            <p className="pt-1 text-[11px] leading-relaxed text-tyba-text-faint">
              {t("reviewAgentHint")}
            </p>
            <div className="mt-2 divide-y divide-tyba-border overflow-hidden rounded-[6px] border border-tyba-border">
              <SettingRow label={t("reviewAgent")}>
                <Select
                  value={
                    reviewAgent === "claude" || reviewAgent === "codex"
                      ? reviewAgent
                      : "custom"
                  }
                  onChange={(v) =>
                    onReviewAgentChange(v === "custom" ? "" : v)
                  }
                  className="w-56"
                  options={[
                    { value: "claude", label: "Claude Code" },
                    { value: "codex", label: "Codex" },
                    { value: "custom", label: t("reviewAgentCustom") },
                  ]}
                />
              </SettingRow>
              {reviewAgent !== "claude" && reviewAgent !== "codex" && (
                <SettingRow
                  label={t("reviewAgentCustomCommand")}
                  hint={t("reviewAgentCustomHint")}
                >
                  <TextField
                    value={reviewAgent}
                    placeholder="claude --model opus"
                    onCommit={onReviewAgentChange}
                  />
                </SettingRow>
              )}
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
            <div className="divide-y divide-tyba-border overflow-hidden rounded-[8px] border border-tyba-border">
              <SettingRow
                label={t("sidebarToggleBehavior", {
                  combo: formatCombo(bindings.panel),
                })}
              >
                <Select
                  value={togglePref}
                  onChange={(v) => onTogglePrefChange(v as SidebarTogglePref)}
                  className="w-48"
                  options={[
                    { value: "hidden", label: t("collapseAll") },
                    { value: "rail", label: t("collapseRail") },
                  ]}
                />
              </SettingRow>
              <SettingRow label={t("sidebarDetails")}>
                <Switch
                  checked={detailsPref === "on"}
                  onCheckedChange={(c) => onDetailsPrefChange(c ? "on" : "off")}
                />
              </SettingRow>
              <SettingRow
                label={t("gitStatusToggle")}
                hint={t("gitStatusHint")}
              >
                <Switch
                  checked={showGitStatus}
                  onCheckedChange={onShowGitStatusChange}
                />
              </SettingRow>
              <SettingRow
                label={t("worktreeDefaultPref")}
                hint={t("worktreeDefaultHint")}
              >
                <Switch
                  checked={worktreeDefault}
                  onCheckedChange={onWorktreeDefaultChange}
                />
              </SettingRow>
              <SettingRow
                label={t("toolbarToggle")}
                hint={t("toolbarHint")}
              >
                <Switch
                  checked={toolbarPref.enabled}
                  onCheckedChange={(c) =>
                    onToolbarPrefChange({ ...toolbarPref, enabled: c })
                  }
                />
              </SettingRow>
              {RICH_INPUT_TOGGLES.map(({ field, label, hint }) => (
                <SettingRow
                  key={field}
                  label={t(label)}
                  hint={
                    hint
                      ? t(hint, { combo: formatCombo(bindings.richInput) })
                      : undefined
                  }
                >
                  <Switch
                    checked={richInputPref[field]}
                    onCheckedChange={(c) =>
                      onRichInputPrefChange({ ...richInputPref, [field]: c })
                    }
                  />
                </SettingRow>
              ))}
              <SettingRow
                label={t("richInputRegex")}
                hint={
                  richInputRegexInvalid
                    ? t("richInputRegexInvalid")
                    : t("richInputRegexHint")
                }
              >
                <div className="w-64">
                  <TextField
                    value={richInputPref.agentRegex}
                    placeholder="^(claude|codex|gemini)\b"
                    onCommit={(v) =>
                      onRichInputPrefChange({
                        ...richInputPref,
                        agentRegex: v,
                      })
                    }
                  />
                </div>
              </SettingRow>
              <SettingRow label={t("language")}>
                <Select
                  value={i18n.language}
                  onChange={(v) => setLanguage(v as LanguageCode)}
                  className="w-48"
                  options={LANGUAGES.map((l) => ({
                    value: l.code,
                    label: l.label,
                  }))}
                />
              </SettingRow>
            </div>
            {toolbarPref.enabled && (
              <ToolbarChipsEditor pref={toolbarPref} onChange={onToolbarPrefChange} />
            )}
          </section>
        )}
      </div>
    </div>
  );
}

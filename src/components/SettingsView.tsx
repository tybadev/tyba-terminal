import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { searchSettings } from "../lib/settingsSearch";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { changelogUrl } from "@/lib/changelog";
import {
  ArrowSquareOut,
  DownloadSimple,
  FolderOpen,
  Info,
  Lock,
  ShieldCheck,
  MagnifyingGlass,
  FileText,
  Bell,
  CaretDoubleLeft,
  CaretDoubleRight,
  GearSix,
  PlugsConnected,
  Robot,
  X,
  Keyboard,
  Palette,
  TerminalWindow,
  User,
} from "@phosphor-icons/react";

import appIcon from "../../src-tauri/icons/128x128.png";

import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { SettingGroup, SettingLine, SettingNote } from "./settings/rows";
import { ThemePicker } from "./settings/ThemePicker";
import { IS_MAC } from "@/lib/platform";
import { NOTIFY_KINDS, notifyEnabled } from "@/lib/notifyPrefs";
import { ClaudePluginSetting } from "./ClaudePluginSetting";
import { AppIconPicker } from "./AppIconPicker";
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
  getEffectiveBase,
  getThemeMode,
  onEffectiveBaseChange,
  onThemeModeChange,
  setThemeMode,
  type Theme,
  type ThemeMode,
} from "../theme";
import {
  appBuildInfo,
  getPref,
  getThemeState,
  importThemeCmd,
  listEditors,
  listThemes,
  onThemeChanged,
  setPref,
  type BuildInfo,
  type EditorInfo,
  type ThemeState,
  type UpdateStatus,
  type LaunchConfig,
  type LaunchConfigId,
  userConfigPath,
  sandboxOverview,
  type SandboxOverview,
} from "../lib/ipc";
import {
  formatDiagnostics,
  platformLabel,
  shortCommitDate,
} from "../lib/diagnostics";
import { openExternalUrl, writeClipboardText } from "../lib/clipboard";
import {
  commitUrl,
  docsUrl,
  LICENSE_NAME,
  LICENSE_URL,
  REPO_URL,
} from "../lib/links";
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
import {
  FONT_SIZE_EVENT,
  getDefaultFontSize,
  setDefaultFontSize,
} from "./TerminalView";
import { ShellSettings } from "./ShellSettings";
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
  | "terminal"
  | "agents"
  | "security"
  | "integrations"
  | "notifications"
  | "shortcuts"
  | "about";

/** As seções, na ordem do trilho: cada uma é um assunto, não uma gaveta. */
const SECTIONS: { id: Section; label: string; hint: string; icon: React.ReactNode }[] = [
  { id: "general", label: "settingsGeneral", hint: "settingsGeneralShort", icon: <User size={16} /> },
  { id: "appearance", label: "settingsAppearance", hint: "settingsAppearanceShort", icon: <Palette size={16} /> },
  { id: "terminal", label: "settingsTerminal", hint: "settingsTerminalShort", icon: <TerminalWindow size={16} /> },
  { id: "agents", label: "settingsAgents", hint: "settingsAgentsShort", icon: <Robot size={16} /> },
  { id: "security", label: "settingsSecurity", hint: "settingsSecurityShort", icon: <ShieldCheck size={16} /> },
  { id: "integrations", label: "settingsIntegrations", hint: "settingsIntegrationsShort", icon: <PlugsConnected size={16} /> },
  { id: "notifications", label: "settingsNotifications", hint: "settingsNotificationsShort", icon: <Bell size={16} /> },
  { id: "shortcuts", label: "settingsShortcuts", hint: "settingsShortcutsShort", icon: <Keyboard size={16} /> },
  { id: "about", label: "settingsAbout", hint: "settingsAboutShort", icon: <Info size={16} /> },
];

const RAIL_PREF_KEY = "pref.settings_rail";

/** As regras que o core impõe e que nenhuma configuração desliga. */
const SECURITY_RULES = [
  "securityRuleRed",
  "securityRuleMain",
  "securityRuleEnv",
  "securityRuleSecrets",
  "securityRuleKill",
];

/**
 * Segurança: o que é fixo aparece como leitura, para ninguém procurar um
 * interruptor que não existe; a única folga da jaula (`read_allow`) se lê aqui
 * e se edita no arquivo — afrouxar a jaula nunca é um clique.
 */
function SecuritySection({ onOpenConfig }: { onOpenConfig: () => void }) {
  const { t } = useTranslation();
  const [overview, setOverview] = useState<SandboxOverview | null>(null);
  useEffect(() => {
    void sandboxOverview()
      .then(setOverview)
      .catch(() => {});
  }, []);
  return (
    <>
      <SettingGroup title={t("securityFixed")} note={t("securityFixedNote")} />
      {SECURITY_RULES.map((key) => (
        <SettingLine key={key} label={t(key)} hint={t(`${key}Hint`)}>
          <span className="flex items-center gap-1.5 text-[11px] text-tyba-text-faint">
            <Lock size={12} />
            {t("securityAlwaysOn")}
          </span>
        </SettingLine>
      ))}
      <SettingGroup
        title={t("securityJail")}
        note={overview && !overview.error ? t("securityReadAllowCount", { count: overview.read_allow.length }) : undefined}
      />
      <SettingLine label={t("securityReadAllow")} hint={t("securityReadAllowHint")}>
        <Button variant="ghost" size="sm" onClick={onOpenConfig}>
          {t("settingsOpenInTyba")}
        </Button>
      </SettingLine>
      {overview?.error && <SettingNote tone="amber">{overview.error}</SettingNote>}
      {overview && !overview.error && overview.read_allow.length === 0 && (
        <SettingNote>{t("securityReadAllowNone")}</SettingNote>
      )}
      {overview?.read_allow.map((path) => (
        <SettingNote key={path} tone="muted">
          <span className="font-mono">{path}</span>
        </SettingNote>
      ))}
      <SettingGroup title={t("securityRepo")} />
      <SettingNote>{t("securityRepoHint")}</SettingNote>
    </>
  );
}

/**
 * O que a busca conhece, por seção: chaves de i18n do nome. A descrição é a
 * chave `<nome>Hint` quando existe. Atalhos e notificações entram inteiros.
 */
// Função e não constante: as listas que ela usa moram mais abaixo no módulo.
const searchKeys = (): Record<Section, string[]> => ({
  general: ["account", "defaultSessionDir", "startup", "language"],
  appearance: [
    "colorMode",
    "fontSection",
    "settingsThemes",
    "appIconSection",
    "settingsSidebar",
    "sidebarDetails",
    "gitStatusToggle",
    "settingsChipsBar",
    "toolbarToggle",
  ],
  terminal: [
    "promptModeTitle",
    "shellIntegration",
    "settingsHistory",
    "historyEnabled",
    "historyImportTitle",
    "historyClear",
    "settingsSnippets",
    ...RICH_INPUT_TOGGLES.map((r) => r.label),
  ],
  agents: ["reviewAgent", "worktreeDefaultPref", "richInputRegex", "settingsLaunchConfigs"],
  security: [...SECURITY_RULES, "securityReadAllow", "securityRepo"],
  integrations: ["dockerIntegration", "claudePlugin", "defaultEditor"],
  notifications: NOTIFY_KINDS.map((k) => k.label),
  shortcuts: Object.values(ACTION_LABEL_KEYS),
  about: ["version", "aboutCopyDiagnostics"],
});

/** `/Users/x/.tyba/config.toml` → `~/.tyba/config.toml`. */
function homeShort(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+/, "~");
}

/**
 * O trilho de seções, à direita como nas outras telas cheias. Aberto mostra o
 * nome; recolhido fica só o ícone, com o nome no tooltip.
 */
function SettingsRail({
  section,
  onSection,
  collapsed,
  onToggle,
  onClose,
}: {
  section: Section;
  onSection: (section: Section) => void;
  collapsed: boolean;
  onToggle: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const item = (
    key: string,
    icon: React.ReactNode,
    label: string,
    tip: string,
    onClick: () => void,
    active = false,
    trailing?: React.ReactNode,
  ) => (
    <Tooltip key={key}>
      <TooltipTrigger asChild>
        <button
          onClick={onClick}
          aria-label={label}
          className={`flex h-8 shrink-0 items-center gap-2.5 rounded-[7px] text-[12.5px] transition-colors ${
            collapsed ? "w-9 justify-center" : "px-2.5"
          } ${
            active
              ? "bg-tyba-green/[.12] text-tyba-green"
              : "text-tyba-text-muted hover:bg-tyba-text/[.04] hover:text-tyba-text"
          }`}
        >
          <span className="shrink-0">{icon}</span>
          {!collapsed && (
            <span className={`min-w-0 flex-1 truncate text-left ${active ? "text-tyba-text" : ""}`}>
              {label}
            </span>
          )}
          {!collapsed && trailing}
        </button>
      </TooltipTrigger>
      {collapsed && (
        <TooltipContent side="left">
          {label}
          {tip && <span className="text-tyba-text-faint"> · {tip}</span>}
        </TooltipContent>
      )}
    </Tooltip>
  );
  return (
    <nav
      aria-label={t("settings")}
      className={`flex shrink-0 flex-col gap-0.5 border-l border-tyba-border bg-tyba-raised px-2 pt-3 pb-2.5 ${
        collapsed ? "w-[52px] items-center" : "w-[188px]"
      }`}
    >
      {SECTIONS.map((s) =>
        item(s.id, s.icon, t(s.label), t(s.hint), () => onSection(s.id), s.id === section),
      )}
      <span className="flex-1" />
      {item(
        "toggle",
        collapsed ? <CaretDoubleLeft size={14} /> : <CaretDoubleRight size={14} />,
        collapsed ? t("settingsRailExpand") : t("settingsRailCollapse"),
        "",
        onToggle,
      )}
      {item(
        "close",
        <X size={15} />,
        t("close"),
        "Esc",
        onClose,
        false,
        <span className="text-[11px] text-tyba-text-ghost">Esc</span>,
      )}
    </nav>
  );
}

interface Props {
  /** Fecha a tela e volta para onde estava. */
  onClose: () => void;
  /** Abre o `config.toml` no editor do TYBA. */
  onOpenConfig: () => void;
  launchConfigs: LaunchConfig[];
  onApplyLaunchConfig: (id: LaunchConfigId) => void;
  onEditLaunchConfig: (id: LaunchConfigId) => void;
  onDeleteLaunchConfig: (id: LaunchConfigId) => void;
  onNewLaunchConfig: () => void;
  onRefreshLaunchConfigs: () => void;
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


const MAC_SOUNDS = ["Ping", "Glass", "Submarine", "Hero", "Funk", "Blow"];


/**
 * Aviso do sistema por espécie de evento.
 *
 * Lê e grava a própria preferência em vez de subir por prop: quem consome estes
 * valores é o core, na hora de notificar — nenhuma outra parte do React precisa
 * saber deles, e passá-los pelo `App` só somaria plumbing.
 */
function NotifyPrefs() {
  const { t } = useTranslation();
  // O estado nasce nos defaults de fábrica, e não vazio: entre a montagem e a
  // resposta do `getPref` a linha já aparece, e uma espécie que nasce desligada
  // não pode piscar ligada nesse intervalo.
  const [enabled, setEnabled] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(NOTIFY_KINDS.map((k) => [k.id, k.defaultEnabled])),
  );
  const [sound, setSound] = useState<Record<string, string>>({});

  useEffect(() => {
    for (const kind of NOTIFY_KINDS) {
      void getPref(kind.enabledKey)
        .then((v) =>
          setEnabled((prev) => ({
            ...prev,
            [kind.id]: notifyEnabled(v, kind.defaultEnabled),
          })),
        )
        .catch(() => {});
      void getPref(kind.soundKey)
        .then((v) =>
          // Ausente é "nunca escolhi": a UI mostra o som de fábrica, que é o
          // que o core vai tocar.
          setSound((prev) => ({ ...prev, [kind.id]: v ?? kind.defaultSound })),
        )
        .catch(() => {});
    }
  }, []);

  return (
    <>
      {NOTIFY_KINDS.map((kind) => (
        <SettingLine key={kind.id} label={t(kind.label)} hint={t(kind.hint)}>
          <div className="flex items-center gap-2">
            {IS_MAC && enabled[kind.id] && (
              <Select
                value={sound[kind.id] ?? kind.defaultSound}
                onChange={(v) => {
                  setSound((prev) => ({ ...prev, [kind.id]: v }));
                  void setPref(kind.soundKey, v).catch(() => {});
                }}
                className="w-36"
                options={[
                  ...MAC_SOUNDS.map((name) => ({ value: name, label: name })),
                  { value: "", label: t("notifySilent") },
                ]}
              />
            )}
            <Switch
              checked={enabled[kind.id] ?? kind.defaultEnabled}
              onCheckedChange={(c) => {
                setEnabled((prev) => ({ ...prev, [kind.id]: c }));
                void setPref(kind.enabledKey, c ? "on" : "off").catch(() => {});
              }}
            />
          </div>
        </SettingLine>
      ))}
    </>
  );
}

function ExternalRow({
  label,
  value,
  href,
  mono,
}: {
  label: string;
  value: string;
  href: string;
  mono?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <SettingLine label={label}>
      <button
        onClick={() => void openExternalUrl(href).catch(() => {})}
        aria-label={t("aboutOpenLink", { target: value })}
        className="flex items-center gap-1.5 text-[12px] text-tyba-text-muted transition-colors hover:text-tyba-text"
      >
        <span className={mono ? "font-mono" : undefined}>{value}</span>
        <ArrowSquareOut size={12} />
      </button>
    </SettingLine>
  );
}

function AboutSection({
  version,
  update,
}: {
  version: string;
  update: UpdateStatus | null;
}) {
  const { t, i18n } = useTranslation();
  const [info, setInfo] = useState<BuildInfo | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let alive = true;
    void appBuildInfo()
      .then((value) => {
        if (alive) setInfo(value);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = () => {
    if (!info) return;
    void writeClipboardText(formatDiagnostics(info, i18n.language))
      .then(() => setCopied(true))
      .catch(() => {});
  };

  const commitDate = info ? shortCommitDate(info.commit_date, i18n.language) : "";

  return (
    <section className="mx-auto w-full max-w-[860px]">
      <div className="flex items-center gap-4 px-[18px] pt-5 pb-4">
        <img src={appIcon} alt="" className="size-[52px] rounded-[12px]" draggable={false} />
        <div className="flex min-w-0 flex-col gap-1">
          <span className="font-display text-[20px] font-bold text-tyba-text">TYBA {version || ""}</span>
          <span className="text-[12px] text-tyba-text-faint">
            {t("aboutTagline")} ·{" "}
            {update ? (
              <span className="text-tyba-violet">{t("updateAvailable", { version: update.info.version })}</span>
            ) : (
              <span className="text-tyba-green">{t("aboutUpToDateShort")}</span>
            )}
          </span>
        </div>
      </div>
      <SettingGroup
        title={t("aboutDiagnostics")}
        note={
          <span className="flex items-center gap-2">
            {copied && (
              <span role="status" className="text-tyba-green">
                {t("aboutCopied")}
              </span>
            )}
            <button
              disabled={!info}
              onClick={copy}
              aria-label={t("aboutCopyDiagnostics")}
              className="text-tyba-blue hover:text-tyba-text disabled:opacity-50"
            >
              {t("aboutCopyDiagnostics")}
            </button>
          </span>
        }
      />
      {info?.commit ? (
        <ExternalRow label={t("aboutCommit")} value={info.commit} href={commitUrl(info.commit)} mono />
      ) : (
        <SettingLine label={t("aboutCommit")}>
          <span className="font-mono text-[12px] text-tyba-text-faint">—</span>
        </SettingLine>
      )}
      <SettingLine label={t("aboutCommitDate")}>
        <span className="text-[12px] text-tyba-text-muted">{commitDate || "—"}</span>
      </SettingLine>
      <SettingLine label={t("aboutPlatform")}>
        <span className="text-[12px] text-tyba-text-muted">{info ? platformLabel(info) : "—"}</span>
      </SettingLine>
      <SettingLine label={t("aboutWebview")}>
        <span className="text-[12px] text-tyba-text-muted">{info?.webview || "—"}</span>
      </SettingLine>
      <SettingGroup title={t("aboutLinks")} />
      <ExternalRow label={t("aboutRepo")} value="github.com/tybadev/tyba-terminal" href={REPO_URL} />
      <ExternalRow label={t("aboutDocs")} value="docs.tyba.dev" href={docsUrl(i18n.language)} />
      <ExternalRow label={t("aboutChangelog")} value="tyba.dev/changelog" href={changelogUrl(i18n.language)} />
      <ExternalRow label={t("aboutLicense")} value={LICENSE_NAME} href={LICENSE_URL} />
    </section>
  );
}

/** Versão e atualização, no fim de Geral. */
function UpdateRows({ version, update }: { version: string; update: UpdateStatus | null }) {
  const { t, i18n } = useTranslation();
  return (
    <>
      <SettingGroup title={t("settingsUpdates")} />
      <SettingLine
        label={`TYBA ${version || "—"}`}
        hint={update ? t("updateAvailable", { version: update.info.version }) : t("updateUpToDate")}
      >
        <Button
          variant="ghost"
          size="sm"
          className={update ? "text-tyba-violet" : "text-tyba-text-muted"}
          onClick={() => void openExternalUrl(changelogUrl(i18n.language)).catch(() => {})}
        >
          {update && <DownloadSimple size={13} />}
          {t("updateOpenChangelog")}
        </Button>
      </SettingLine>
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
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
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
    <div
      data-setting-row
      className="grid grid-cols-[1fr_auto] items-center gap-4 border-b border-tyba-border-soft px-[18px] py-[5px] text-[12.5px]"
    >
      <span className="min-w-0 truncate text-tyba-text">
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
  onClose,
  onOpenConfig,
  launchConfigs,
  onApplyLaunchConfig,
  onEditLaunchConfig,
  onDeleteLaunchConfig,
  onNewLaunchConfig,
  onRefreshLaunchConfigs,
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
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [configPath, setConfigPath] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const contentRef = useRef<HTMLDivElement>(null);
  const searchEntries = useMemo(
    () =>
      SECTIONS.flatMap(({ id }) =>
        searchKeys()[id].map((key) => ({
          section: id,
          label: t(key, { combo: "" }),
          hint: i18n.exists(`${key}Hint`) ? t(`${key}Hint`, { combo: "" }) : undefined,
        })),
      ),
    [t, i18n],
  );
  const results = useMemo(() => searchSettings(searchEntries, query), [searchEntries, query]);
  // Da busca para a linha: troca de seção e, depois de ela desenhar, rola até
  // o texto e acende a linha por um instante.
  const goToSetting = (section: Section, label: string) => {
    setQuery("");
    setSection(section);
    requestAnimationFrame(() => {
      const root = contentRef.current;
      if (!root) return;
      const hit = Array.from(root.querySelectorAll<HTMLElement>("p, span, label, button")).find(
        (el) => el.childElementCount === 0 && el.textContent?.trim() === label.trim(),
      );
      const row = (hit?.closest("[data-setting-row]") as HTMLElement | null) ?? hit;
      if (!row) return;
      row.scrollIntoView({ block: "center" });
      row.classList.add("tyba-setting-flash");
      window.setTimeout(() => row.classList.remove("tyba-setting-flash"), 1200);
    });
  };
  useEffect(() => {
    void userConfigPath()
      .then(setConfigPath)
      .catch(() => {});
  }, []);
  useEffect(() => {
    void getPref(RAIL_PREF_KEY)
      .then((v) => setRailCollapsed(v === "collapsed"))
      .catch(() => {});
  }, []);
  const toggleRail = () => {
    setRailCollapsed((was) => {
      void setPref(RAIL_PREF_KEY, was ? "open" : "collapsed").catch(() => {});
      return !was;
    });
  };
  // Esc fecha, como nas outras telas cheias — a não ser que o foco esteja
  // num campo (aí o Esc é dele) ou gravando um atalho.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || captureState.active) return;
      const el = document.activeElement;
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return;
      onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    if (section !== "agents") return;
    onRefreshLaunchConfigs();
  }, [section, onRefreshLaunchConfigs]);
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
  // O tamanho pode mudar por fora (edição do ~/.tyba/config.toml).
  useEffect(() => {
    const sync = () => setFontSize(getDefaultFontSize());
    window.addEventListener(FONT_SIZE_EVENT, sync);
    return () => window.removeEventListener(FONT_SIZE_EVENT, sync);
  }, []);
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
    // O anúncio vem de dentro do `setDefaultFontSize` — ele é a fonte única.
    setDefaultFontSize(size);
    void setPref("pref.code.font_size", String(size)).catch(() => {});
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


  const current = SECTIONS.find((item) => item.id === section) ?? SECTIONS[0];

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-[46px] shrink-0 items-center gap-3 border-b border-tyba-border bg-tyba-raised px-[18px] text-[12px]">
          <GearSix size={16} className="shrink-0 text-tyba-green" />
          <span className="text-[14px] font-medium text-tyba-text">{t("settings")}</span>
          <span className="min-w-0 flex-1 truncate text-tyba-text-faint">
            {t(current.label)} · {t(current.hint)}
          </span>
          <label className="flex w-[260px] shrink-0 items-center gap-2 rounded-[6px] border border-tyba-border-field px-2.5 py-1 text-tyba-text-faint focus-within:border-tyba-border-strong">
            <MagnifyingGlass size={13} className="shrink-0" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  setQuery("");
                  (e.target as HTMLInputElement).blur();
                }
                if (e.key === "Enter" && results[0]) {
                  goToSetting(results[0].section, results[0].label);
                }
              }}
              placeholder={t("settingsSearch")}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              className="min-w-0 flex-1 bg-transparent text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-ghost"
            />
          </label>
        </header>
        <div
          ref={contentRef}
          className="tyba-settings min-h-0 flex-1 overflow-y-auto bg-tyba-sunken px-8 pt-4 pb-8"
        >
        {query.trim() && (
          <section className="mx-auto w-full max-w-[860px]">
            <span className="tyba-label">
              {t("settingsSearchResults", { count: results.length })}
            </span>
            <div className="mt-2 flex flex-col">
              {results.length === 0 && (
                <span className="py-3 text-[12px] text-tyba-text-faint">
                  {t("settingsSearchEmpty")}
                </span>
              )}
              {results.map((r) => (
                <button
                  key={`${r.section}:${r.label}`}
                  onClick={() => goToSetting(r.section, r.label)}
                  className="flex flex-col gap-0.5 rounded-[6px] px-2.5 py-2 text-left hover:bg-tyba-text/[.04]"
                >
                  <span className="text-[13px] text-tyba-text">
                    {r.label}
                    <span className="text-[11px] text-tyba-text-faint">
                      {" "}· {t(SECTIONS.find((x) => x.id === r.section)?.label ?? "")}
                    </span>
                  </span>
                  {r.hint && (
                    <span className="line-clamp-2 text-[11px] leading-relaxed text-tyba-text-faint">
                      {r.hint}
                    </span>
                  )}
                </button>
              ))}
            </div>
          </section>
        )}
        {!query.trim() && (<>
        {section === "general" && (
          <section className="mx-auto w-full max-w-[860px]">
            <SettingGroup title={t("account")} />
            <SettingLine label={t("localAccount")} hint={t("accountHint")}>
              <span className="rounded-full p-px" style={{ background: "var(--tyba-gradient)" }}>
                <span className="flex size-7 items-center justify-center rounded-full bg-tyba-raised text-tyba-text-muted">
                  <User size={13} weight="bold" />
                </span>
              </span>
              <div className="w-[220px]">
                <TextField
                  value={accountName}
                  placeholder={t("accountNamePlaceholder")}
                  onCommit={onAccountNameChange}
                />
              </div>
            </SettingLine>
            <SettingGroup title={t("settingsOnOpen")} />
            <SettingLine label={t("startup")} hint={t("startupHint")}>
              <Select
                value={startup}
                onChange={(value) => onStartupChange(parseStartupMode(value))}
                className="w-52"
                options={[
                  { value: "resume", label: t("startupResume") },
                  { value: "keep_layout", label: t("startupKeepLayout") },
                  { value: "fresh", label: t("startupFresh") },
                ]}
              />
            </SettingLine>
            <SettingLine label={t("defaultSessionDir")} hint={t("defaultSessionDirHint")}>
              <div className="w-[200px]">
                <TextField value={defaultDir} placeholder="~" onCommit={commitDefaultDir} />
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void chooseDefaultDir()}
                className="text-tyba-text-muted hover:text-tyba-text"
              >
                <FolderOpen size={14} />
                {t("chooseFolder")}
              </Button>
            </SettingLine>
            <SettingGroup title={t("language")} />
            <SettingLine label={t("language")} hint={t("languageHint")}>
              <Select
                value={i18n.language}
                onChange={(v) => setLanguage(v as LanguageCode)}
                className="w-52"
                options={LANGUAGES.map((l) => ({ value: l.code, label: l.label }))}
              />
            </SettingLine>
            <UpdateRows version={version} update={update} />
          </section>
        )}
        {section === "appearance" && (
          <section className="mx-auto w-full max-w-[860px]">
            <SettingGroup
              title={t("settingsThemes")}
              note={
                <span className="flex items-center gap-2">
                  {t("themesCount", { count: themes.length })} ·
                  <button onClick={() => copyThemeExample()} className="text-tyba-blue hover:text-tyba-text">
                    {t("copyThemeExample")}
                  </button>
                </span>
              }
            />
            <ThemePicker
              themes={themes}
              state={themeState}
              mode={mode}
              effectiveBase={effBase}
              onMode={setThemeMode}
              onImport={() => void importTheme()}
            />
            <SettingGroup title={t("fontSection")} />
            <SettingLine label={t("fontSection")} hint={t("fontHint")}>
              <Select
                value={uiFont}
                onChange={(v) => setUiFont(v as UiFont)}
                className="w-44"
                options={UI_FONTS.map((f) => ({ value: f, label: UI_FONT_LABELS[f] }))}
              />
              <Select
                value={String(fontSize)}
                onChange={(v) => changeFontSize(Number(v))}
                className="w-24"
                options={FONT_SIZES.map((s) => ({ value: String(s), label: `${s}px` }))}
              />
            </SettingLine>
            <div className="border-b border-tyba-border-soft px-[18px] py-3">
              <AppIconPicker />
            </div>
            <SettingGroup title={t("settingsSidebar")} />
            <SettingLine label={t("sidebarToggleBehavior", { combo: formatCombo(bindings.panel) })}>
              <Select
                value={togglePref}
                onChange={(v) => onTogglePrefChange(v as SidebarTogglePref)}
                className="w-44"
                options={[
                  { value: "hidden", label: t("collapseAll") },
                  { value: "rail", label: t("collapseRail") },
                ]}
              />
            </SettingLine>
            <SettingLine label={t("sidebarDetails")}>
              <Switch
                checked={detailsPref === "on"}
                onCheckedChange={(c) => onDetailsPrefChange(c ? "on" : "off")}
              />
            </SettingLine>
            <SettingLine label={t("gitStatusToggle")} hint={t("gitStatusHint")}>
              <Switch checked={showGitStatus} onCheckedChange={onShowGitStatusChange} />
            </SettingLine>
            <SettingGroup title={t("settingsChipsBar")} note={t("chipsDragNote")} />
            <SettingLine label={t("toolbarToggle")} hint={t("toolbarHint")}>
              <Switch
                checked={toolbarPref.enabled}
                onCheckedChange={(c) => onToolbarPrefChange({ ...toolbarPref, enabled: c })}
              />
            </SettingLine>
            {toolbarPref.enabled && (
              <div className="px-[18px] pb-3">
                <ToolbarChipsEditor pref={toolbarPref} onChange={onToolbarPrefChange} />
              </div>
            )}
          </section>
        )}
        {section === "terminal" && (
          <section className="mx-auto w-full max-w-[860px]">
            <ShellSettings
              lineRows={
                <>
                  {RICH_INPUT_TOGGLES.map(({ field, label, hint }) => (
                    <SettingLine
                      key={field}
                      label={t(label)}
                      hint={hint ? t(hint, { combo: formatCombo(bindings.richInput) }) : undefined}
                    >
                      <Switch
                        checked={richInputPref[field]}
                        onCheckedChange={(c) => onRichInputPrefChange({ ...richInputPref, [field]: c })}
                      />
                    </SettingLine>
                  ))}
                  <SettingLine label={t("shellIntegration")} hint={t("shellIntegrationHint")}>
                    <Switch
                      checked={shellIntegration}
                      onCheckedChange={onShellIntegrationChange}
                      aria-label={t("shellIntegration")}
                    />
                  </SettingLine>
                </>
              }
            />
          </section>
        )}
        {section === "agents" && (
          <section className="mx-auto w-full max-w-[860px]">
            <SettingGroup title={t("settingsReview")} />
            <SettingLine label={t("reviewAgent")} hint={t("reviewAgentHint")}>
              <Select
                value={reviewAgent === "claude" || reviewAgent === "codex" ? reviewAgent : "custom"}
                onChange={(v) => onReviewAgentChange(v === "custom" ? "" : v)}
                className="w-52"
                options={[
                  { value: "claude", label: "Claude Code" },
                  { value: "codex", label: "Codex" },
                  { value: "custom", label: t("reviewAgentCustom") },
                ]}
              />
            </SettingLine>
            {reviewAgent !== "claude" && reviewAgent !== "codex" && (
              <SettingLine label={t("reviewAgentCustomCommand")} hint={t("reviewAgentCustomHint")}>
                <div className="w-64">
                  <TextField value={reviewAgent} placeholder="claude --model opus" onCommit={onReviewAgentChange} />
                </div>
              </SettingLine>
            )}
            <SettingGroup title={t("settingsSessions")} />
            <SettingLine label={t("worktreeDefaultPref")} hint={t("worktreeDefaultHint")}>
              <Switch checked={worktreeDefault} onCheckedChange={onWorktreeDefaultChange} />
            </SettingLine>
            <SettingLine
              label={t("richInputRegex")}
              hint={richInputRegexInvalid ? t("richInputRegexInvalid") : t("richInputRegexHint")}
            >
              <div className="w-60 font-mono">
                <TextField
                  value={richInputPref.agentRegex}
                  placeholder="^(claude|codex|gemini)\b"
                  onCommit={(v) => onRichInputPrefChange({ ...richInputPref, agentRegex: v })}
                />
              </div>
            </SettingLine>
            <SettingGroup
              title={t("settingsLaunchConfigs")}
              note={
                <button onClick={onNewLaunchConfig} className="text-tyba-blue hover:text-tyba-text">
                  + {t("launchConfigNew")}
                </button>
              }
            />
            {launchConfigs.length === 0 && <SettingNote>{t("launchNoConfigs")}</SettingNote>}
            {launchConfigs.map((config) => (
              <SettingLine
                key={config.id}
                label={config.name}
                hint={`${config.repo_root} · ${config.slots.length} ${config.slots.length === 1 ? "pane" : "panes"}`}
              >
                <Button variant="ghost" size="sm" onClick={() => onApplyLaunchConfig(config.id)}>
                  {t("launchOpen")}
                </Button>
                <Button variant="ghost" size="sm" onClick={() => onEditLaunchConfig(config.id)}>
                  {t("launchEdit")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-tyba-red"
                  onClick={() => onDeleteLaunchConfig(config.id)}
                >
                  {t("launchDelete")}
                </Button>
              </SettingLine>
            ))}
          </section>
        )}
        {section === "security" && (
          <section className="mx-auto w-full max-w-[860px]">
            <SecuritySection onOpenConfig={onOpenConfig} />
          </section>
        )}
        {section === "integrations" && (
          <section className="mx-auto w-full max-w-[860px]">
            <SettingGroup title="Docker" />
            <SettingLine label={t("dockerIntegration")} hint={t("dockerIntegrationHint")}>
              <Switch
                checked={showContainers}
                onCheckedChange={onShowContainersChange}
                aria-label={t("dockerIntegration")}
              />
            </SettingLine>
            <SettingGroup title="Claude Code" />
            <ClaudePluginSetting />
            <SettingGroup title={t("settingsEditor")} />
            <SettingLine label={t("defaultEditor")} hint={editorPath ?? t("defaultEditorHint")}>
              <Select
                value={editor}
                onChange={onEditorChange}
                className="w-52"
                options={[
                  { value: "", label: t("defaultEditorSystem") },
                  ...editors.map((e) => ({ value: e.id, label: e.name })),
                ]}
              />
            </SettingLine>
          </section>
        )}
        {section === "notifications" && (
          <section className="mx-auto w-full max-w-[860px]">
            <SettingGroup title={t("settingsAgents")} />
            <NotifyPrefs />
          </section>
        )}
        {section === "shortcuts" && (
          <section className="mx-auto w-full max-w-[860px]">
            {actionsByCategory().map(([category, actions], i) =>
              actions.length === 0 ? null : (
                <div key={category}>
                  <SettingGroup
                    title={t(KEY_CATEGORY_LABEL_KEYS[category])}
                    note={i === 0 ? t("shortcutsClickToChange") : undefined}
                  />
                  {actions.map((action) => (
                    <ShortcutRow
                      key={action}
                      action={action}
                      combo={bindings[action]}
                      onRebind={(combo) => onBindingsChange({ ...bindings, [action]: combo })}
                    />
                  ))}
                </div>
              ),
            )}
          </section>
        )}
        {section === "about" && (
          <AboutSection version={version} update={update} />
        )}
        </>)}
        </div>
        {configPath && (
          <footer className="flex h-[26px] shrink-0 items-center gap-2.5 border-t border-tyba-border bg-tyba-raised px-[18px] text-[11px] text-tyba-text-faint">
            <FileText size={12} className="shrink-0" />
            <span className="truncate">
              {t("settingsFileIn")}{" "}
              <span className="text-tyba-text-muted">{homeShort(configPath)}</span>
            </span>
            <button
              onClick={onOpenConfig}
              className="shrink-0 text-tyba-blue hover:text-tyba-text"
            >
              {t("settingsOpenInTyba")}
            </button>
            <span className="flex-1" />
            <span className="hidden truncate lg:inline">{t("settingsFileLive")}</span>
          </footer>
        )}
      </div>
      <SettingsRail
        section={section}
        onSection={setSection}
        collapsed={railCollapsed}
        onToggle={toggleRail}
        onClose={onClose}
      />
    </div>
  );
}

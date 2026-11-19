import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { searchSettings } from "../lib/settingsSearch";
import { Trans, useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { changelogUrl } from "@/lib/changelog";
import {
  ArrowClockwise,
  Bell,
  CaretDoubleLeft,
  CaretDoubleRight,
  CircleHalf,
  Code,
  Copy,
  DownloadSimple,
  FolderOpen,
  Gear,
  Globe,
  Keyboard,
  MagnifyingGlass,
  PenNib,
  Play,
  Plug,
  Plus,
  ShieldCheck,
  Trash,
  User,
  X,
} from "@phosphor-icons/react";

import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  SETTING_FIELD,
  SettingButton,
  SettingFixed,
  SettingGroup,
  SettingLine,
  SettingNote,
  SettingSegment,
  SettingStatus,
} from "./settings/rows";
import { fileLinksEnabled, setFileLinksEnabled } from "../lib/fileLinks";
import { ThemePicker } from "./settings/ThemePicker";
import { IS_MAC } from "@/lib/platform";
import { NOTIFY_KINDS, notifyEnabled } from "@/lib/notifyPrefs";
import { ClaudePluginSetting } from "./ClaudePluginSetting";
import { SshSection } from "./settings/SshSection";
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
  dockerAvailable,
  openNotificationSettings,
  previewNotifySound,
  updateCheck,
} from "../lib/ipc";
import { formatDiagnostics, platformLabel } from "../lib/diagnostics";
import { openExternalUrl, writeClipboardText } from "../lib/clipboard";
import { commitUrl, docsUrl, LICENSE_NAME, LICENSE_URL, REPO_URL } from "../lib/links";
import {
  actionsByCategory,
  ACTION_LABEL_KEYS,
  captureState,
  comboOf,
  formatCombo,
  KEY_CATEGORY_LABEL_KEYS,
  type Bindings,
  type KeyAction,
  DEFAULT_BINDINGS,
} from "../lib/keys";
import { Shortcut } from "@/components/ui/kbd";
import { toastError } from "../lib/toast";
import { FONT_SIZE_EVENT, getDefaultFontSize, setDefaultFontSize } from "./TerminalView";
import { ShellSettings } from "./ShellSettings";
import { ToolbarChipsEditor } from "./ToolbarChipsEditor";
import type { RichInputPref } from "../lib/richInput";
import { DEFAULT_TOOLBAR, type ToolbarPref } from "../lib/repoSnapshots";
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
  | "notifications"
  | "integrations"
  | "security"
  | "shortcuts"
  | "about";

/**
 * As seções, na ordem da lista do quadro "Configurações (aprovada)". Atalhos
 * não está no quadro; fica antes de Sobre para não sumir.
 */
const SECTIONS: { id: Section; label: string; sub: string; icon: React.ReactNode }[] = [
  { id: "general", label: "settingsGeneral", sub: "cfgSubGeneral", icon: <Gear size={16} /> },
  { id: "appearance", label: "settingsAppearance", sub: "cfgSubAppearance", icon: <CircleHalf size={16} /> },
  { id: "terminal", label: "settingsTerminal", sub: "cfgSubTerminal", icon: <Code size={16} /> },
  { id: "agents", label: "settingsAgents", sub: "cfgSubAgents", icon: <User size={16} /> },
  { id: "notifications", label: "settingsNotifications", sub: "cfgSubNotifications", icon: <Bell size={16} /> },
  { id: "integrations", label: "settingsIntegrations", sub: "cfgSubIntegrations", icon: <Plug size={16} /> },
  { id: "security", label: "settingsSecurity", sub: "cfgSubSecurity", icon: <ShieldCheck size={16} /> },
  { id: "shortcuts", label: "settingsShortcuts", sub: "shortcutsHint", icon: <Keyboard size={16} /> },
  { id: "about", label: "settingsAbout", sub: "cfgSubAbout", icon: <Globe size={16} /> },
];

const RAIL_PREF_KEY = "pref.settings_rail";

/** As regras que o core impõe e que nenhuma configuração desliga. */
const SECURITY_RULES = ["cfgRuleMain", "cfgRuleSecrets", "cfgRuleKill"];

/**
 * Segurança: o que é fixo aparece como leitura (cadeado + "Fixo"), para
 * ninguém procurar um interruptor que não existe; o agente roda livre.
 */
function SecuritySection() {
  const { t } = useTranslation();
  return (
    <>
      <SettingGroup title={t("cfgGroupAlways")}>
        {SECURITY_RULES.map((key) => (
          <SettingLine key={key} label={t(key)} hint={t(`${key}Hint`)}>
            <SettingFixed>{t("cfgFixed")}</SettingFixed>
          </SettingLine>
        ))}
      </SettingGroup>
      <SettingGroup title={t("securityAgent")}>
        <SettingLine label={t("cfgRuleFree")} hint={t("cfgRuleFreeHint")} />
        <SettingLine
          label={t("cfgRepoConfig")}
          hint={
            <Trans
              i18nKey="cfgRepoConfigHint"
              components={{ file: <span className="font-mono text-[12.5px] text-tyba-text" /> }}
            />
          }
        />
      </SettingGroup>
    </>
  );
}

/**
 * O que a busca conhece, por seção: chaves de i18n do nome. A descrição é a
 * chave `<nome>Hint` quando existe. Atalhos entram inteiros.
 */
// Função e não constante: as listas que ela usa moram mais abaixo no módulo.
const searchKeys = (): Record<Section, string[]> => ({
  general: ["cfgName", "language", "cfgStartup", "cfgSessionDir"],
  appearance: [
    "colorMode",
    "settingsThemes",
    "cfgFont",
    "cfgSize",
    "cfgCollapse",
    "cfgGitStatus",
    "sidebarDetails",
    "cfgFooter",
    "cfgFooterArrange",
    "appIconSection",
  ],
  terminal: [
    "promptModeTitle",
    "cfgAutoOpen",
    "autocompleteShowUses",
    "cfgZsh",
    "cfgHistoryOn",
    "cfgHistoryImport",
    "cfgHistoryClear",
    "settingsSnippets",
  ],
  agents: [
    ...COMPOSER_TOGGLES.map((r) => r.label),
    "cfgSendWith",
    "cfgAgentCommands",
    "richInputShowOnMatch",
    "worktreeDefaultPref",
    "cfgReviewer",
    "cfgGroupLaunch",
  ],
  notifications: [...NOTIFY_KINDS.map((k) => NOTIFY_LABELS[k.id] ?? k.label), "cfgMacos"],
  integrations: ["cfgClaudeCode", "cfgDocker", "cfgGroupSsh", "sshTypedDetect", "sshTypedWhen", "cfgEditor", "cfgFileLinks"],
  security: [...SECURITY_RULES, "cfgRuleFree", "cfgRepoConfig"],
  shortcuts: Object.values(ACTION_LABEL_KEYS),
  about: ["version", "aboutDiagnostics", "aboutDocs", "aboutWhatChanged"],
});

/** `/Users/x/.tyba/config.toml` → `~/.tyba/config.toml`. */
function homeShort(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+/, "~");
}

/**
 * A lista de seções, à direita: ícone e nome; o escolhido ganha a mancha azul
 * atrás do ícone e o nome em negrito, sem pílula. Fechar fica embaixo, com o
 * Esc. Recolhida fica só o ícone, com o nome no tooltip.
 */
function SettingsRail({
  section,
  onSection,
  collapsed,
  onToggle,
  onClose,
  search,
}: {
  section: Section;
  onSection: (section: Section) => void;
  collapsed: boolean;
  onToggle: () => void;
  onClose: () => void;
  search: React.ReactNode;
}) {
  const { t } = useTranslation();
  const item = (
    key: string,
    icon: React.ReactNode,
    label: string,
    onClick: () => void,
    active = false,
    trailing?: React.ReactNode,
  ) => (
    <Tooltip key={key}>
      <TooltipTrigger asChild>
        <button
          type="button"
          onClick={onClick}
          aria-label={label}
          aria-current={active ? "page" : undefined}
          className={`flex h-9 shrink-0 items-center gap-2.5 rounded-[10px] text-left text-[13px] transition-colors focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)] ${
            collapsed ? "w-10 justify-center" : "px-2"
          } ${active ? "font-bold text-tyba-text" : "text-tyba-text-muted hover:text-tyba-text"}`}
        >
          <span
            className={`grid size-[26px] shrink-0 place-items-center rounded-[8px] ${
              active
                ? "bg-tyba-blue-tint text-tyba-blue shadow-[0_0_14px_2px_color-mix(in_srgb,var(--tyba-blue)_18%,transparent)]"
                : ""
            }`}
          >
            {icon}
          </span>
          {!collapsed && <span className="min-w-0 flex-1 truncate">{label}</span>}
          {!collapsed && trailing}
        </button>
      </TooltipTrigger>
      {collapsed && <TooltipContent side="left">{label}</TooltipContent>}
    </Tooltip>
  );
  return (
    <nav
      aria-label={t("cfgSections")}
      className={`flex shrink-0 flex-col gap-0.5 border-l border-[var(--tyba-line)] px-2.5 py-3 ${
        collapsed ? "w-[60px] items-center" : "w-[208px]"
      }`}
    >
      {!collapsed && <div className="pb-2">{search}</div>}
      {SECTIONS.map((s) => item(s.id, s.icon, t(s.label), () => onSection(s.id), s.id === section))}
      <span className="flex-1" />
      {item(
        "toggle",
        collapsed ? <CaretDoubleLeft size={14} /> : <CaretDoubleRight size={14} />,
        collapsed ? t("settingsRailExpand") : t("settingsRailCollapse"),
        onToggle,
      )}
      <div className="mx-1.5 my-2 h-px self-stretch bg-[var(--tyba-line)]" />
      {item(
        "close",
        <X size={16} />,
        t("close"),
        onClose,
        false,
        <kbd className="ml-auto font-[inherit] text-[11.5px] text-tyba-text-faint">Esc</kbd>,
      )}
    </nav>
  );
}

interface Props {
  /** Fecha a tela e volta para onde estava. */
  onClose: () => void;
  /**
   * Abre (ou leva) a tela direto nesta seção — o "Ajustar" do toast da troca
   * SSH pede `ssh`, que hoje mora em Integrações › Hosts SSH.
   */
  initialSection?: Section | "ssh";
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
  field: keyof Omit<RichInputPref, "version" | "agentRegex" | "submitWithCtrlEnter" | "showOnMatch">;
  label: string;
  hint?: string;
};

/** Caixa de prompt: os do quadro primeiro (o "Enviar com" vai depois do primeiro). */
const COMPOSER_TOGGLES: RichInputToggle[] = [
  { field: "autoShow", label: "cfgAutoShow", hint: "cfgAutoShowHint" },
  { field: "warnOnSensitivePrompt", label: "cfgWarnKey", hint: "cfgWarnKeyHint" },
  { field: "autoOpenOnStart", label: "richInputAutoOpen" },
  { field: "autoDismiss", label: "richInputAutoDismiss" },
];

/** O nome do quadro para as espécies de aviso que ele mostra. */
const NOTIFY_LABELS: Record<string, string> = {
  request: "cfgNotifyRequest",
  done: "cfgNotifyDone",
};

/** Nome curto da espécie na linha de som. */
const NOTIFY_SOUND_LABELS: Record<string, string> = {
  request: "cfgSoundRequest",
  done: "cfgSoundDone",
  observedRequest: "cfgSoundObserved",
};

/** Os três tamanhos do quadro; outro valor (do config.toml) aparece como px. */
const TEXT_SIZES = [
  { value: 12, label: "cfgSizeSmall" },
  { value: 13, label: "cfgSizeMedium" },
  { value: 15, label: "cfgSizeLarge" },
];

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

  const withSound = NOTIFY_KINDS.filter((kind) => enabled[kind.id]);

  return (
    <>
      <SettingGroup title={t("cfgGroupWhen")}>
        {NOTIFY_KINDS.map((kind) => {
          const label = NOTIFY_LABELS[kind.id] ?? kind.label;
          return (
            <SettingLine key={kind.id} label={t(label)} hint={t(`${label}Hint`)}>
              <Switch
                checked={enabled[kind.id] ?? kind.defaultEnabled}
                aria-label={t(label)}
                onCheckedChange={(c) => {
                  setEnabled((prev) => ({ ...prev, [kind.id]: c }));
                  void setPref(kind.enabledKey, c ? "on" : "off").catch(() => {});
                }}
              />
            </SettingLine>
          );
        })}
      </SettingGroup>
      {IS_MAC && withSound.length > 0 && (
        <SettingGroup title={t("cfgGroupSound")}>
          {withSound.map((kind) => {
            const current = sound[kind.id] ?? kind.defaultSound;
            return (
              <SettingLine key={kind.id} label={t(NOTIFY_SOUND_LABELS[kind.id] ?? kind.label)}>
                {current && (
                  <SettingButton
                    tone="ghost"
                    icon={<Play size={16} />}
                    onClick={() => void previewNotifySound(current).catch(() => {})}
                  >
                    {t("cfgListen")}
                  </SettingButton>
                )}
                <Select
                  value={current}
                  onChange={(v) => {
                    setSound((prev) => ({ ...prev, [kind.id]: v }));
                    void setPref(kind.soundKey, v).catch(() => {});
                  }}
                  className={SETTING_FIELD}
                  options={[
                    ...MAC_SOUNDS.map((name) => ({ value: name, label: name })),
                    { value: "", label: t("notifySilent") },
                  ]}
                />
              </SettingLine>
            );
          })}
        </SettingGroup>
      )}
      {IS_MAC && (
        <SettingGroup title={t("cfgGroupOutside")}>
          <SettingLine label={t("cfgMacos")} hint={t("cfgMacosHint")}>
            <SettingButton onClick={() => void openNotificationSettings().catch(() => {})}>
              {t("notifyMacosOpen")}
            </SettingButton>
          </SettingLine>
        </SettingGroup>
      )}
    </>
  );
}

/** Versão (com a busca de atualização), diagnóstico e links. */
function AboutSection({ version, update }: { version: string; update: UpdateStatus | null }) {
  const { t, i18n } = useTranslation();
  const [info, setInfo] = useState<BuildInfo | null>(null);
  const [copied, setCopied] = useState(false);
  // O resultado de "Procurar atualização" vale mais que o do boot: é mais novo.
  const [checked, setChecked] = useState<UpdateStatus | null | undefined>(undefined);
  const [checking, setChecking] = useState(false);
  const current = checked === undefined ? update : checked;

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

  const check = () => {
    setChecking(true);
    void updateCheck()
      .then(setChecked)
      .catch((e) => toastError(t("updateCheckFailed"), e))
      .finally(() => setChecking(false));
  };

  const copy = () => {
    if (!info) return;
    void writeClipboardText(formatDiagnostics(info, i18n.language))
      .then(() => setCopied(true))
      .catch(() => {});
  };

  const open = (href: string) => void openExternalUrl(href).catch(() => {});
  const build = info?.commit || `${version || "—"} (${import.meta.env.DEV ? "dev" : "release"})`;

  return (
    <>
      <SettingGroup title={t("version")}>
        <SettingLine
          label={`TYBA ${version || "—"}`}
          hint={current ? t("updateAvailable", { version: current.info.version }) : t("cfgUpToDate")}
        >
          <SettingButton icon={<ArrowClockwise size={16} />} disabled={checking} onClick={check}>
            {checking ? t("updateChecking") : t("cfgCheckUpdate")}
          </SettingButton>
          <SettingButton
            tone="ghost"
            icon={current ? <DownloadSimple size={16} /> : undefined}
            className={current ? "text-tyba-violet" : ""}
            onClick={() => open(changelogUrl(i18n.language))}
          >
            {t("updateOpenChangelog")}
          </SettingButton>
        </SettingLine>
        <SettingLine
          label={t("aboutDiagnostics")}
          hint={
            <Trans
              i18nKey="cfgDiagHint"
              values={{ build, platform: info ? platformLabel(info) : "—" }}
              components={{
                commit: info?.commit ? (
                  <button
                    type="button"
                    onClick={() => open(commitUrl(info.commit))}
                    aria-label={t("aboutOpenLink", { target: info.commit })}
                    className="font-mono hover:text-tyba-text"
                  />
                ) : (
                  <span />
                ),
              }}
            />
          }
        >
          {copied && (
            <span role="status" className="text-[13px] font-semibold text-tyba-green">
              {t("aboutCopied")}
            </span>
          )}
          <SettingButton
            tone="ghost"
            icon={<Copy size={16} />}
            disabled={!info}
            onClick={copy}
            aria-label={t("aboutCopyDiagnostics")}
          >
            {t("cfgCopy")}
          </SettingButton>
        </SettingLine>
      </SettingGroup>
      <SettingGroup title={t("cfgGroupProject")}>
        <SettingLine label={t("cfgOpenSource", { license: LICENSE_NAME })} hint="github.com/tybadev/tyba-terminal">
          <SettingButton tone="ghost" onClick={() => open(LICENSE_URL)}>
            {t("aboutLicense")}
          </SettingButton>
          <SettingButton tone="ghost" onClick={() => open(REPO_URL)}>
            {t("cfgOpen")}
          </SettingButton>
        </SettingLine>
        <SettingLine label={t("aboutDocs")} hint="docs.tyba.dev">
          <SettingButton tone="ghost" onClick={() => open(docsUrl(i18n.language))}>
            {t("cfgOpen")}
          </SettingButton>
        </SettingLine>
        <SettingLine label={t("aboutWhatChanged")} hint="changelog">
          <SettingButton tone="ghost" onClick={() => open(changelogUrl(i18n.language))}>
            {t("cfgOpen")}
          </SettingButton>
        </SettingLine>
      </SettingGroup>
    </>
  );
}

function TextField({
  value,
  placeholder,
  onCommit,
  ariaLabel,
  className = "",
}: {
  value: string;
  placeholder: string;
  onCommit: (value: string) => void;
  ariaLabel: string;
  className?: string;
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
      aria-label={ariaLabel}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => onCommit(draft.trim())}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
      }}
      className={`${SETTING_FIELD} px-3 text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:shadow-[0_0_0_1px_var(--tyba-border-strong)] ${className}`}
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
    <SettingLine label={t(ACTION_LABEL_KEYS[action])}>
      <button
        type="button"
        onClick={() => setListening(true)}
        className={`flex h-[30px] items-center gap-1 rounded-[8px] px-2 text-[12px] transition-colors ${
          listening
            ? "font-mono text-tyba-blue shadow-[inset_0_0_0_1px_var(--tyba-blue)]"
            : "hover:bg-tyba-hover"
        }`}
      >
        {listening ? t("pressKeys") : <Shortcut combo={combo} />}
      </button>
    </SettingLine>
  );
}

export function SettingsView({
  onClose,
  initialSection,
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
  // O SSH de antes mora em Integrações › Hosts SSH.
  const toSection = (s: Section | "ssh"): Section => (s === "ssh" ? "integrations" : s);
  const [section, setSection] = useState<Section>(initialSection ? toSection(initialSection) : "general");
  useEffect(() => {
    if (initialSection) setSection(toSection(initialSection));
  }, [initialSection]);
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [configPath, setConfigPath] = useState<string | null>(null);
  const [dockerUp, setDockerUp] = useState<boolean | null>(null);
  const [filesInTyba, setFilesInTyba] = useState(fileLinksEnabled);
  useEffect(() => {
    if (section !== "integrations" || !showContainers) return;
    void dockerAvailable()
      .then(setDockerUp)
      .catch(() => setDockerUp(false));
  }, [section, showContainers]);
  const [query, setQuery] = useState("");
  const contentRef = useRef<HTMLDivElement>(null);
  const searchEntries = useMemo(
    () =>
      SECTIONS.flatMap(({ id }) =>
        searchKeys()[id].map((key) => ({
          section: id,
          label: t(key, { combo: formatCombo(bindings.panel), license: LICENSE_NAME, name: "" }),
          // Sem as marcações do `<Trans>` (`<file>…</file>`): a busca mostra texto.
          hint: i18n.exists(`${key}Hint`)
            ? t(`${key}Hint`, { combo: formatCombo(bindings.richInput) }).replace(/<\/?[a-z]+>/g, "")
            : undefined,
        })),
      ),
    [t, i18n, bindings.panel, bindings.richInput],
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
      const hit = Array.from(root.querySelectorAll<HTMLElement>("p, span, label, button, h2")).find(
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
  useEffect(() => onEffectiveBaseChange(() => setEffBase(getEffectiveBase())), []);

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
  const searching = query.trim().length > 0;
  const sizeOptions = [
    ...TEXT_SIZES.map((s) => ({ value: String(s.value), label: t(s.label) })),
    ...(TEXT_SIZES.some((s) => s.value === fontSize) ? [] : [{ value: String(fontSize), label: `${fontSize}px` }]),
  ];
  const reviewChoice = reviewAgent === "claude" || reviewAgent === "codex" ? reviewAgent : "custom";

  const search = (
    <label className="flex h-8 items-center gap-2 rounded-[10px] px-2.5 text-tyba-text-faint shadow-[inset_0_0_0_1px_var(--tyba-line)] focus-within:shadow-[inset_0_0_0_1px_var(--tyba-border-strong)]">
      <MagnifyingGlass size={14} className="shrink-0" />
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
        placeholder={t("cfgSearch")}
        aria-label={t("settingsSearch")}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        className="min-w-0 flex-1 bg-transparent text-[12.5px] text-tyba-text outline-none placeholder:text-tyba-text-ghost"
      />
    </label>
  );

  return (
    <div className="flex min-h-0 flex-1 justify-center text-tyba-text" style={{ background: "var(--tyba-field)" }}>
      {/* Coluna de ajustes e lista de seções juntas, num bloco centrado: em
          janela larga a lista não vai parar no canto e não sobra um vazio no
          meio (dono, 2026-11-19: "tá feião"). */}
      <div className="flex min-h-0 w-full max-w-[1040px]">
      <div ref={contentRef} className="tyba-settings min-h-0 min-w-0 flex-1 overflow-y-auto px-14 pt-9 pb-12">
        <div className="max-w-[680px]">
          <div className="flex items-end justify-between gap-6">
            <div className="min-w-0">
              <h1 className="m-0 text-[28px] leading-[34px] font-bold tracking-[-.025em]">
                {searching ? t("settings") : t(current.label)}
              </h1>
              <p className="mt-1.5 mb-0 text-[14px] leading-[21px] text-tyba-text-muted">
                {searching ? t("settingsSearchResults", { count: results.length }) : t(current.sub)}
              </p>
            </div>
            {!searching && section === "appearance" && (
              <SettingSegment<ThemeMode>
                label={t("colorMode")}
                options={[
                  { value: "light", label: t("themeLight") },
                  { value: "dark", label: t("themeDark") },
                  { value: "system", label: t("themeSystem") },
                ]}
                value={mode}
                onChange={setThemeMode}
              />
            )}
          </div>

          {searching && (
            <div className="mt-6 flex flex-col">
              {results.length === 0 && (
                <span className="py-3 text-[13px] text-tyba-text-faint">{t("settingsSearchEmpty")}</span>
              )}
              {results.map((r) => (
                <button
                  key={`${r.section}:${r.label}`}
                  type="button"
                  onClick={() => goToSetting(r.section, r.label)}
                  className="flex flex-col gap-0.5 border-b border-[var(--tyba-line)] py-[13px] text-left last:border-b-0 hover:text-tyba-blue"
                >
                  <span className="text-[14px] leading-5 font-semibold">
                    {r.label}
                    <span className="text-[12.5px] font-normal text-tyba-text-faint">
                      {" "}
                      · {t(SECTIONS.find((x) => x.id === r.section)?.label ?? "")}
                    </span>
                  </span>
                  {r.hint && (
                    <span className="line-clamp-2 max-w-[420px] text-[13px] leading-[19px] text-tyba-text-muted">
                      {r.hint}
                    </span>
                  )}
                </button>
              ))}
            </div>
          )}

          {!searching && section === "general" && (
            <>
              <SettingGroup title={t("cfgGroupYou")}>
                <SettingLine label={t("cfgName")} hint={t("cfgNameHint")}>
                  <TextField
                    value={accountName}
                    placeholder={t("accountNamePlaceholder")}
                    onCommit={onAccountNameChange}
                    ariaLabel={t("cfgName")}
                  />
                </SettingLine>
                <SettingLine label={t("language")}>
                  <Select
                    value={i18n.language}
                    onChange={(v) => setLanguage(v as LanguageCode)}
                    className={SETTING_FIELD}
                    options={LANGUAGES.map((l) => ({ value: l.code, label: l.label }))}
                  />
                </SettingLine>
              </SettingGroup>
              <SettingGroup title={t("cfgGroupOnOpen")}>
                <SettingLine label={t("cfgStartup")} hint={t("cfgStartupHint")}>
                  <SettingSegment<StartupMode>
                    label={t("cfgStartup")}
                    options={[
                      { value: "resume", label: t("cfgStartupResume") },
                      { value: "keep_layout", label: t("startupKeepLayout") },
                      { value: "fresh", label: t("cfgStartupFresh") },
                    ]}
                    value={startup}
                    onChange={(value) => onStartupChange(parseStartupMode(value))}
                  />
                </SettingLine>
                <SettingLine label={t("cfgSessionDir")} hint={t("cfgSessionDirHint")}>
                  <TextField
                    value={defaultDir}
                    placeholder="~"
                    onCommit={commitDefaultDir}
                    ariaLabel={t("cfgSessionDir")}
                  />
                  <SettingButton
                    tone="ghost"
                    className="px-2.5"
                    onClick={() => void chooseDefaultDir()}
                    aria-label={t("chooseFolder")}
                    icon={<FolderOpen size={16} />}
                  />
                </SettingLine>
              </SettingGroup>
            </>
          )}

          {!searching && section === "appearance" && (
            <>
              <ThemePicker
                themes={themes}
                state={themeState}
                mode={mode}
                effectiveBase={effBase}
                fontSize={fontSize}
                footer={toolbarPref.enabled}
                onImport={() => void importTheme()}
                onCopyExample={copyThemeExample}
              />
              <SettingGroup title={t("cfgGroupText")}>
                <SettingLine label={t("cfgFont")} hint={t("cfgFontHint")}>
                  <Select
                    value={uiFont}
                    onChange={(v) => setUiFont(v as UiFont)}
                    className={SETTING_FIELD}
                    options={UI_FONTS.map((f) => ({ value: f, label: UI_FONT_LABELS[f] }))}
                  />
                </SettingLine>
                <SettingLine label={t("cfgSize")}>
                  <SettingSegment
                    label={t("cfgSize")}
                    options={sizeOptions}
                    value={String(fontSize)}
                    onChange={(v) => changeFontSize(Number(v))}
                  />
                </SettingLine>
              </SettingGroup>
              <SettingGroup title={t("cfgGroupSidebar")}>
                <SettingLine
                  label={t("cfgCollapse", { combo: formatCombo(bindings.panel) })}
                  hint={t("cfgCollapseHint")}
                >
                  <SettingSegment<SidebarTogglePref>
                    label={t("cfgCollapse", { combo: formatCombo(bindings.panel) })}
                    options={[
                      { value: "hidden", label: t("cfgCollapseHidden") },
                      { value: "rail", label: t("cfgCollapseRail") },
                    ]}
                    value={togglePref}
                    onChange={onTogglePrefChange}
                  />
                </SettingLine>
                <SettingLine label={t("cfgGitStatus")} hint={t("cfgGitStatusHint")}>
                  <Switch
                    checked={showGitStatus}
                    onCheckedChange={onShowGitStatusChange}
                    aria-label={t("cfgGitStatus")}
                  />
                </SettingLine>
                <SettingLine label={t("sidebarDetails")}>
                  <Switch
                    checked={detailsPref === "on"}
                    onCheckedChange={(c) => onDetailsPrefChange(c ? "on" : "off")}
                    aria-label={t("sidebarDetails")}
                  />
                </SettingLine>
                <SettingLine label={t("cfgFooter")} hint={t("cfgFooterHint")}>
                  <Switch
                    checked={toolbarPref.enabled}
                    onCheckedChange={(c) => onToolbarPrefChange({ ...toolbarPref, enabled: c })}
                    aria-label={t("cfgFooter")}
                  />
                </SettingLine>
                {toolbarPref.enabled && (
                  <SettingLine
                    label={t("cfgFooterArrange")}
                    hint={t("cfgFooterArrangeHint")}
                    below={<ToolbarChipsEditor pref={toolbarPref} onChange={onToolbarPrefChange} />}
                  >
                    <SettingButton
                      tone="ghost"
                      icon={<ArrowClockwise size={16} />}
                      onClick={() => onToolbarPrefChange({ ...DEFAULT_TOOLBAR, enabled: toolbarPref.enabled })}
                    >
                      {t("chipsEditorReset")}
                    </SettingButton>
                  </SettingLine>
                )}
              </SettingGroup>
              <SettingGroup title={t(IS_MAC ? "cfgGroupIconMac" : "appIconSection")}>
                <AppIconPicker />
              </SettingGroup>
            </>
          )}

          {!searching && section === "terminal" && (
            <ShellSettings
              lineRows={
                <SettingLine label={t("cfgZsh")} hint={t("cfgZshHint")}>
                  <Switch
                    checked={shellIntegration}
                    onCheckedChange={onShellIntegrationChange}
                    aria-label={t("cfgZsh")}
                  />
                </SettingLine>
              }
            />
          )}

          {!searching && section === "agents" && (
            <>
              <SettingGroup title={t("cfgGroupComposer")}>
                {COMPOSER_TOGGLES.slice(0, 1).map(({ field, label, hint }) => (
                  <SettingLine
                    key={field}
                    label={t(label)}
                    hint={hint ? t(hint, { combo: formatCombo(bindings.richInput) }) : undefined}
                  >
                    <Switch
                      checked={richInputPref[field]}
                      aria-label={t(label)}
                      onCheckedChange={(c) => onRichInputPrefChange({ ...richInputPref, [field]: c })}
                    />
                  </SettingLine>
                ))}
                <SettingLine
                  label={t("cfgSendWith")}
                  hint={richInputPref.submitWithCtrlEnter ? t("cfgSendCtrlEnterHint") : t("cfgSendEnterHint")}
                >
                  <SettingSegment
                    label={t("cfgSendWith")}
                    options={[
                      { value: "enter", label: "Enter" },
                      { value: "ctrl", label: "Ctrl Enter" },
                    ]}
                    value={richInputPref.submitWithCtrlEnter ? "ctrl" : "enter"}
                    onChange={(v) => onRichInputPrefChange({ ...richInputPref, submitWithCtrlEnter: v === "ctrl" })}
                  />
                </SettingLine>
                {COMPOSER_TOGGLES.slice(1).map(({ field, label, hint }) => (
                  <SettingLine key={field} label={t(label)} hint={hint ? t(hint) : undefined}>
                    <Switch
                      checked={richInputPref[field]}
                      aria-label={t(label)}
                      onCheckedChange={(c) => onRichInputPrefChange({ ...richInputPref, [field]: c })}
                    />
                  </SettingLine>
                ))}
              </SettingGroup>
              <SettingGroup title={t("cfgGroupDetect")}>
                <SettingLine
                  label={t("cfgAgentCommands")}
                  hint={richInputRegexInvalid ? t("richInputRegexInvalid") : t("richInputRegexHint")}
                >
                  <TextField
                    value={richInputPref.agentRegex}
                    placeholder="^(claude|codex|gemini)\b"
                    onCommit={(v) => onRichInputPrefChange({ ...richInputPref, agentRegex: v })}
                    ariaLabel={t("cfgAgentCommands")}
                    className="font-mono text-[12.5px]"
                  />
                </SettingLine>
                <SettingLine label={t("richInputShowOnMatch")} hint={t("richInputShowOnMatchHint")}>
                  <Switch
                    checked={richInputPref.showOnMatch}
                    aria-label={t("richInputShowOnMatch")}
                    onCheckedChange={(c) => onRichInputPrefChange({ ...richInputPref, showOnMatch: c })}
                  />
                </SettingLine>
                <SettingLine label={t("worktreeDefaultPref")} hint={t("worktreeDefaultHint")}>
                  <Switch
                    checked={worktreeDefault}
                    onCheckedChange={onWorktreeDefaultChange}
                    aria-label={t("worktreeDefaultPref")}
                  />
                </SettingLine>
              </SettingGroup>
              <SettingGroup title={t("cfgGroupReview")}>
                <SettingLine label={t("cfgReviewer")} hint={t("cfgReviewerHint")}>
                  <SettingSegment
                    label={t("cfgReviewer")}
                    options={[
                      { value: "claude", label: "Claude Code" },
                      { value: "codex", label: "Codex" },
                      { value: "custom", label: t("cfgReviewerCustom") },
                    ]}
                    value={reviewChoice}
                    onChange={(v) => onReviewAgentChange(v === "custom" ? "" : v)}
                  />
                </SettingLine>
                {reviewChoice === "custom" && (
                  <SettingLine label={t("reviewAgentCustomCommand")} hint={t("reviewAgentCustomHint")}>
                    <TextField
                      value={reviewAgent}
                      placeholder="claude --model opus"
                      onCommit={onReviewAgentChange}
                      ariaLabel={t("reviewAgentCustomCommand")}
                      className="font-mono text-[12.5px]"
                    />
                  </SettingLine>
                )}
              </SettingGroup>
              <SettingGroup title={t("cfgGroupLaunch")}>
                {launchConfigs.length === 0 && <SettingNote>{t("launchNoConfigs")}</SettingNote>}
                {launchConfigs.map((config) => (
                  <SettingLine
                    key={config.id}
                    label={config.name}
                    hint={`${config.repo_root} · ${config.slots.length} ${config.slots.length === 1 ? "pane" : "panes"}`}
                  >
                    <SettingButton tone="ghost" icon={<Play size={16} />} onClick={() => onApplyLaunchConfig(config.id)}>
                      {t("launchOpen")}
                    </SettingButton>
                    <SettingButton tone="ghost" icon={<PenNib size={16} />} onClick={() => onEditLaunchConfig(config.id)}>
                      {t("launchEdit")}
                    </SettingButton>
                    <SettingButton
                      tone="ghost"
                      className="px-2.5 hover:text-tyba-red"
                      aria-label={t("launchDelete")}
                      icon={<Trash size={16} />}
                      onClick={() => onDeleteLaunchConfig(config.id)}
                    />
                  </SettingLine>
                ))}
                <SettingLine>
                  <SettingButton icon={<Plus size={16} />} onClick={onNewLaunchConfig}>
                    {t("cfgLaunchNew")}
                  </SettingButton>
                </SettingLine>
              </SettingGroup>
            </>
          )}

          {!searching && section === "notifications" && <NotifyPrefs />}

          {!searching && section === "integrations" && (
            <>
              <SettingGroup title={t("cfgGroupConnected")}>
                <ClaudePluginSetting />
                <SettingLine label={t("cfgDocker")} hint={t("cfgDockerHint")}>
                  {showContainers && dockerUp !== null && (
                    <SettingStatus tone={dockerUp ? "green" : "faint"}>
                      {dockerUp ? t("dockerRunning") : t("dockerNotFound")}
                    </SettingStatus>
                  )}
                  <Switch
                    checked={showContainers}
                    onCheckedChange={onShowContainersChange}
                    aria-label={t("cfgDocker")}
                  />
                </SettingLine>
              </SettingGroup>
              <SshSection />
              <SettingGroup title={t("settingsEditor")}>
                <SettingLine label={t("cfgEditor")} hint={editorPath ?? t("cfgEditorHint")}>
                  <Select
                    value={editor}
                    onChange={onEditorChange}
                    className={SETTING_FIELD}
                    options={[
                      { value: "", label: t("defaultEditorSystem") },
                      ...editors.map((e) => ({ value: e.id, label: e.name })),
                    ]}
                  />
                </SettingLine>
                <SettingLine label={t("cfgFileLinks", { combo: IS_MAC ? "⌘" : "Ctrl" })} hint={t("cfgFileLinksHint")}>
                  <Switch
                    checked={filesInTyba}
                    aria-label={t("cfgFileLinks", { combo: IS_MAC ? "⌘" : "Ctrl" })}
                    onCheckedChange={(c) => {
                      setFilesInTyba(c);
                      setFileLinksEnabled(c);
                    }}
                  />
                </SettingLine>
              </SettingGroup>
            </>
          )}

          {!searching && section === "security" && <SecuritySection />}

          {!searching && section === "shortcuts" &&
            actionsByCategory().map(([category, actions], i) =>
              actions.length === 0 ? null : (
                <SettingGroup
                  key={category}
                  title={t(KEY_CATEGORY_LABEL_KEYS[category])}
                  note={
                    i === 0 ? (
                      <button
                        type="button"
                        onClick={() => onBindingsChange({ ...DEFAULT_BINDINGS })}
                        className="font-semibold text-tyba-blue hover:underline"
                      >
                        {t("shortcutsRestore")}
                      </button>
                    ) : undefined
                  }
                >
                  {actions.map((action) => (
                    <ShortcutRow
                      key={action}
                      action={action}
                      combo={bindings[action]}
                      onRebind={(combo) => onBindingsChange({ ...bindings, [action]: combo })}
                    />
                  ))}
                </SettingGroup>
              ),
            )}

          {!searching && section === "about" && <AboutSection version={version} update={update} />}

          {configPath && (
            <div className="mt-10 text-[13px] text-tyba-text-muted">
              <Trans
                i18nKey="cfgFileFoot"
                values={{ path: homeShort(configPath) }}
                components={{ file: <span className="font-mono text-[12.5px] text-tyba-text" /> }}
              />
              <button
                type="button"
                onClick={onOpenConfig}
                className="ml-1 font-semibold text-tyba-blue hover:underline"
              >
                {t("cfgFileOpen")}
              </button>
            </div>
          )}
        </div>
      </div>
      <SettingsRail
        section={section}
        onSection={(s) => {
          setQuery("");
          setSection(s);
          contentRef.current?.scrollTo({ top: 0 });
        }}
        collapsed={railCollapsed}
        onToggle={toggleRail}
        onClose={onClose}
        search={search}
      />
      </div>
    </div>
  );
}

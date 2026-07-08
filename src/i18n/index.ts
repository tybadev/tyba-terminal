// i18n do TYBA: pt-BR e en no MVP.
// Idioma: escolha do usuário (localStorage) > idioma do sistema > en.
// Termos de domínio (branch, diff, worktree, merge) ficam em inglês
// em qualquer idioma — são vocabulário técnico, não texto de UI.

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

const STORAGE_KEY = "tyba.lang";

export const LANGUAGES = [
  { code: "pt-BR", label: "Português (Brasil)" },
  { code: "en", label: "English" },
] as const;

export type LanguageCode = (typeof LANGUAGES)[number]["code"];

const resources = {
  "pt-BR": {
    translation: {
      sessions: "Sessões",
      newSession: "Nova sessão",
      closeSession: "Fechar sessão",
      newTab: "Nova tab",
      closeTab: "Fechar tab",
      closePane: "Fechar pane",
      killSession: "Encerrar sessão",
      looseSessions: "Avulsas",
      noTabs: "Nenhuma tab aberta.",
      newSessionWhere: "Onde essa sessão vai trabalhar?",
      lastFolder: "última pasta",
      homeFolder: "Pasta pessoal (~)",
      chooseFolder: "Escolher pasta…",
      settingsAccount: "Conta",
      settingsThemes: "Temas",
      settingsPreferences: "Preferências",
      accountHint: "Contas e sincronização chegam depois.",
      themesHint: "O terminal acompanha o tema do app.",
      preferencesHint: "Comportamentos do TYBA nesta máquina.",
      sidebarToggleBehavior: "Recolher painel (⌘B)",
      collapseAll: "Recolher tudo",
      collapseRail: "Modo ícones",
      themeInUse: "em uso",
      useThemeShort: "Usar",
      sessionEnded: "[sessão encerrada]",
      noSessions: "Nenhuma sessão aberta.",
      hintNewSession: "nova sessão",
      hintPanel: "painel",
      hintPalette: "paleta",
      panelToggle: "Painel: aberto → ícones → oculto",
      openProjectFolder: "Abrir pasta do projeto",
      notifications: "Notificações",
      notificationsEmpty:
        "Tudo em dia. Aprovações de sessões de agente chegam aqui.",
      account: "Conta",
      localAccount: "Conta local",
      settings: "Configurações",
      about: "Sobre o TYBA",
      language: "Idioma",
      commandPalette: "Paleta de comandos",
      searchCommand: "Buscar comando…",
      searchSessions: "Buscar sessão…",
      noResults: "Nada encontrado.",
      hintNavigate: "navegar",
      hintRun: "executar",
      hintClose: "fechar",
      actions: "Ações",
      togglePanel: "Alternar painel",
      switchLanguageTo: "Mudar idioma para {{lang}}",
      theme: "Tema",
      themeDark: "Escuro",
      themeLight: "Claro",
      themeSystem: "Sistema",
      switchThemeTo: "Mudar tema para {{theme}}",
      useTheme: "Usar tema {{name}}",
      importTheme: "Importar tema…",
      themeImportFailed: "Falha ao importar tema: {{error}}",
      approvals: "Aprovações",
      pendingCount_one: "{{count}} pendente",
      pendingCount_other: "{{count}} pendentes",
      approve: "Aprovar",
      deny: "Recusar",
      confirmApprove: "Confirmar",
      redAction: "ação vermelha",
    },
  },
  en: {
    translation: {
      sessions: "Sessions",
      newSession: "New session",
      closeSession: "Close session",
      newTab: "New tab",
      closeTab: "Close tab",
      closePane: "Close pane",
      killSession: "Kill session",
      looseSessions: "Ungrouped",
      noTabs: "No open tabs.",
      newSessionWhere: "Where will this session work?",
      lastFolder: "last folder",
      homeFolder: "Home folder (~)",
      chooseFolder: "Choose folder…",
      settingsAccount: "Account",
      settingsThemes: "Themes",
      settingsPreferences: "Preferences",
      accountHint: "Accounts and sync come later.",
      themesHint: "The terminal follows the app theme.",
      preferencesHint: "TYBA behaviors on this machine.",
      sidebarToggleBehavior: "Collapse panel (⌘B)",
      collapseAll: "Collapse fully",
      collapseRail: "Icon rail",
      themeInUse: "in use",
      useThemeShort: "Use",
      sessionEnded: "[session ended]",
      noSessions: "No open sessions.",
      hintNewSession: "new session",
      hintPanel: "panel",
      hintPalette: "palette",
      panelToggle: "Panel: open → icons → hidden",
      openProjectFolder: "Open project folder",
      notifications: "Notifications",
      notificationsEmpty:
        "All caught up. Agent session approvals arrive here.",
      account: "Account",
      localAccount: "Local account",
      settings: "Settings",
      about: "About TYBA",
      language: "Language",
      commandPalette: "Command palette",
      searchCommand: "Search commands…",
      searchSessions: "Search sessions…",
      hintNavigate: "navigate",
      hintRun: "run",
      hintClose: "close",
      noResults: "No results.",
      actions: "Actions",
      togglePanel: "Toggle panel",
      switchLanguageTo: "Switch language to {{lang}}",
      theme: "Theme",
      themeDark: "Dark",
      themeLight: "Light",
      themeSystem: "System",
      switchThemeTo: "Switch theme to {{theme}}",
      useTheme: "Use {{name}} theme",
      importTheme: "Import theme…",
      themeImportFailed: "Failed to import theme: {{error}}",
      approvals: "Approvals",
      pendingCount_one: "{{count}} pending",
      pendingCount_other: "{{count}} pending",
      approve: "Approve",
      deny: "Deny",
      confirmApprove: "Confirm",
      redAction: "red action",
    },
  },
} as const;

function detectLanguage(): LanguageCode {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved && LANGUAGES.some((l) => l.code === saved)) {
    return saved as LanguageCode;
  }
  return navigator.language?.toLowerCase().startsWith("pt") ? "pt-BR" : "en";
}

export function setLanguage(code: LanguageCode) {
  localStorage.setItem(STORAGE_KEY, code);
  void i18n.changeLanguage(code);
}

void i18n.use(initReactI18next).init({
  resources,
  lng: detectLanguage(),
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;

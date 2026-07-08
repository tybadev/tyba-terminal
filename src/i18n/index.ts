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
      noResults: "Nada encontrado.",
      actions: "Ações",
      togglePanel: "Alternar painel",
      switchLanguageTo: "Mudar idioma para {{lang}}",
    },
  },
  en: {
    translation: {
      sessions: "Sessions",
      newSession: "New session",
      closeSession: "Close session",
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
      noResults: "No results.",
      actions: "Actions",
      togglePanel: "Toggle panel",
      switchLanguageTo: "Switch language to {{lang}}",
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

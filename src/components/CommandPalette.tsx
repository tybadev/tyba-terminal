// Paleta de comandos (⌘K): navegação por teclado como cidadã de
// primeira classe. Ações do shell + salto direto pra qualquer sessão.

import { useTranslation } from "react-i18next";
import {
  Globe,
  Plus,
  SidebarSimple,
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
import type { Session, SessionId } from "../lib/ipc";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sessions: Session[];
  activeId: SessionId | null;
  onNewSession: () => void;
  onCloseActive: () => void;
  onTogglePanel: () => void;
  onGoToSession: (id: SessionId) => void;
}

export function CommandPalette({
  open,
  onOpenChange,
  sessions,
  activeId,
  onNewSession,
  onCloseActive,
  onTogglePanel,
  onGoToSession,
}: Props) {
  const { t, i18n } = useTranslation();

  const run = (fn: () => void) => () => {
    onOpenChange(false);
    fn();
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
              {t("closeSession")}
              <CommandShortcut>⌘W</CommandShortcut>
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

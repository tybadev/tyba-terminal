import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, CircleHalf, Moon, Sun } from "@phosphor-icons/react";

import type { Theme, ThemeMode, ThemeState } from "../../theme";
import { previewTheme, setSlotTheme } from "../../theme";

type Base = "dark" | "light";

/**
 * A escolha de tema do quadro "Configurações · Aparência": modo segmentado,
 * um tema para o escuro e outro para o claro, e cartões que são um mini
 * terminal com as cores de verdade. Mouse em cima pinta o app inteiro; clique
 * fixa.
 */
export function ThemePicker({
  themes,
  state,
  mode,
  effectiveBase,
  onMode,
  onImport,
}: {
  themes: Theme[];
  state: ThemeState | null;
  mode: ThemeMode;
  effectiveBase: Base;
  onMode: (mode: ThemeMode) => void;
  onImport: () => void;
}) {
  const { t } = useTranslation();
  const [slot, setSlot] = useState<Base>(effectiveBase);
  useEffect(() => setSlot(effectiveBase), [effectiveBase]);
  // Sair da tela no meio de uma pré-visualização não pode deixar o app pintado.
  useEffect(() => () => previewTheme(null), []);

  const chosen = state ? state[slot].id : null;
  const shown = themes.filter((theme) => theme.base === slot);

  const segment = <T extends string>(
    items: { id: T; label: React.ReactNode }[],
    value: T,
    onPick: (id: T) => void,
  ) => (
    <span className="inline-flex rounded-[7px] bg-tyba-raised p-0.5 shadow-[inset_0_0_0_1px_var(--tyba-border-field)]">
      {items.map((item) => (
        <button
          key={item.id}
          onClick={() => onPick(item.id)}
          aria-pressed={item.id === value}
          className={`flex items-center gap-1.5 rounded-[5px] px-3 py-1 text-[12px] transition-colors ${
            item.id === value
              ? "bg-tyba-text/[.08] text-tyba-text"
              : "text-tyba-text-faint hover:text-tyba-text-muted"
          }`}
        >
          {item.label}
        </button>
      ))}
    </span>
  );

  return (
    <div className="border-b border-tyba-border-soft px-[18px] pt-3 pb-3.5">
      <div className="flex items-center gap-3.5 pb-2.5">
        {segment<ThemeMode>(
          [
            { id: "dark", label: <><Moon size={13} />{t("themeDark")}</> },
            { id: "light", label: <><Sun size={13} />{t("themeLight")}</> },
            { id: "system", label: <><CircleHalf size={13} />{t("themeSystem")}</> },
          ],
          mode,
          onMode,
        )}
        <span className="flex-1" />
        <span className="text-[11px] text-tyba-text-faint">{t("themeModeNote")}</span>
      </div>
      <div className="flex items-center gap-2 pb-2.5">
        {segment<Base>(
          [
            { id: "dark", label: t("themeSlotDark", { name: state?.dark.name ?? "—" }) },
            { id: "light", label: t("themeSlotLight", { name: state?.light.name ?? "—" }) },
          ],
          slot,
          setSlot,
        )}
        <span className="flex-1" />
        <span className="text-[11px] text-tyba-text-faint">{t("themeHoverNote")}</span>
      </div>
      <div className="grid grid-cols-5 gap-3" onMouseLeave={() => previewTheme(null)}>
        {shown.map((theme) => {
          const on = theme.id === chosen;
          const p = theme.terminal;
          return (
            <button
              key={theme.id}
              onMouseEnter={() => previewTheme(theme)}
              onFocus={() => previewTheme(theme)}
              onBlur={() => previewTheme(null)}
              onClick={() => void setSlotTheme(theme)}
              aria-pressed={on}
              className={`overflow-hidden rounded-[8px] bg-tyba-raised text-left transition-shadow ${
                on
                  ? "shadow-[0_0_0_2px_var(--tyba-green)]"
                  : "shadow-[0_0_0_1px_var(--tyba-border)] hover:shadow-[0_0_0_1px_var(--tyba-border-strong)]"
              }`}
            >
              <div
                aria-hidden
                className="flex h-16 flex-col gap-px px-[9px] py-2 font-mono text-[9.5px] leading-[13px]"
                style={{ background: p.background, color: p.foreground }}
              >
                <span>
                  <span style={{ color: p.ansi[2] }}>❯</span> git status{" "}
                  <span style={{ color: p.ansi[4] }}>main</span>
                </span>
                <span style={{ color: p.ansi[2] }}>+ src/app.tsx</span>
                <span style={{ color: p.ansi[1] }}>− old.ts</span>
                <span>
                  <span style={{ color: p.ansi[3] }}>warn</span> 2
                </span>
              </div>
              <div className="flex items-center gap-1.5 px-[9px] py-1.5 text-[11px]">
                <span className={`min-w-0 flex-1 truncate ${on ? "text-tyba-text" : "text-tyba-text-muted"}`}>
                  {theme.name}
                </span>
                {on && <Check size={11} weight="bold" className="shrink-0 text-tyba-green" />}
              </div>
            </button>
          );
        })}
        <button
          onClick={onImport}
          className="flex min-h-[92px] flex-col items-center justify-center gap-1 rounded-[8px] border border-dashed border-tyba-border-strong text-[11px] text-tyba-text-muted hover:text-tyba-text"
        >
          <span className="text-[16px] text-tyba-text-faint">+</span>
          {t("importTheme")}
          <span className="text-[10px] text-tyba-text-faint">.json</span>
        </button>
      </div>
    </div>
  );
}

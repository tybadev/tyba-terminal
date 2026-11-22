import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "@phosphor-icons/react";

import type { Theme, ThemeMode, ThemeState } from "../../theme";
import { previewTheme, setSlotTheme } from "../../theme";
import { IS_MAC } from "@/lib/platform";

type Base = "dark" | "light";

/** Cor de destaque do tema (o ❯ do prompt e o ponto da pílula). */
const accentOf = (theme: Theme) => theme.ui.primary ?? theme.terminal.ansi[2];

/** A linha fina do terminal na cor do texto do próprio tema. */
const lineOf = (theme: Theme) => `color-mix(in srgb, ${theme.terminal.foreground} 12%, transparent)`;

/** O terminal de verdade, pintado com o tema: a prévia grande da Aparência. */
function TerminalSpecimen({
  theme,
  label,
  fontSize,
  footer,
  selectable,
  selected,
  onPick,
}: {
  theme: Theme;
  label: string;
  fontSize: number;
  footer: boolean;
  selectable: boolean;
  selected: boolean;
  onPick: () => void;
}) {
  const { t } = useTranslation();
  const p = theme.terminal;
  const line = lineOf(theme);
  const accent = accentOf(theme);
  const green = p.ansi[2];
  const red = p.ansi[1];
  const blue = p.ansi[4];
  return (
    <button
      type="button"
      onClick={onPick}
      disabled={!selectable}
      aria-pressed={selectable ? selected : undefined}
      aria-label={selectable ? t("cfgThemePickSlot", { slot: label }) : undefined}
      className={`flex flex-col overflow-hidden rounded-[14px] text-left transition-[background,color,box-shadow] duration-200 focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)] motion-reduce:transition-none disabled:cursor-default ${
        selectable ? "cursor-pointer" : ""
      }`}
      style={{
        background: p.background,
        color: p.foreground,
        boxShadow:
          selectable && selected
            ? `var(--tyba-mk-shadow-float, var(--tyba-shadow-md)), 0 0 0 3px var(--tyba-ring-bg, var(--tyba-bg)), 0 0 0 5px var(--tyba-primary)`
            : `var(--tyba-mk-shadow-float, var(--tyba-shadow-md)), inset 0 0 0 1px var(--tyba-spec-edge, ${line})`,
      }}
    >
      <span className="flex h-[34px] items-center gap-[7px] px-3.5" style={{ borderBottom: `1px solid ${line}` }}>
        {[0, 1, 2].map((i) => (
          <i key={i} className="block size-2.5 rounded-full" style={{ background: line }} />
        ))}
        <span className="ml-2 text-[12px] font-semibold">{label}</span>
        <b className="ml-auto truncate text-[12px] font-semibold opacity-65">{theme.name}</b>
      </span>
      <span
        aria-hidden
        className="flex flex-col px-[18px] pt-4 pb-3.5 font-mono leading-[1.7] whitespace-pre"
        style={{ fontSize }}
      >
        <span>
          <span style={{ color: accent }}>❯</span> claude
        </span>
        <span>
          <span className="opacity-60">✻ {t("cfgSpecEdited")} </span>
          <span style={{ color: green }}>+42</span> <span style={{ color: red }}>−7</span>
        </span>
        <span>
          <span style={{ color: accent }}>❯</span> bun run test
        </span>
        <span>
          <span style={{ color: green }}>✓</span> {t("cfgSpecPassed")}
        </span>
        <span>
          <span style={{ color: accent }}>❯</span> git status
        </span>
        <span>
          <span style={{ color: blue }}>main</span>
          {"  "}
          <span style={{ color: green }}>+ app.tsx</span>
          {"  "}
          <span style={{ color: red }}>− old.ts</span>
        </span>
      </span>
      {footer && (
        <span
          aria-hidden
          className="flex h-7 items-center gap-2 px-3.5 text-[11px]"
          style={{ borderTop: `1px solid ${line}` }}
        >
          <span className="rounded-full px-2 py-0.5 opacity-85" style={{ background: line }}>
            ~/projetos/api
          </span>
          <span className="rounded-full px-2 py-0.5 opacity-85" style={{ background: line }}>
            main
          </span>
          <span className="ml-auto opacity-85">14:52</span>
        </span>
      )}
    </button>
  );
}

/**
 * A escolha de tema do quadro "Configurações (aprovada)": o terminal grande
 * pintado com o tema (dois lado a lado no Sistema, claro à esquerda e escuro
 * à direita; clicar num escolhe qual lado se troca) e as pílulas de tema
 * embaixo. Mouse numa pílula repinta a prévia (e o app, como antes); clique
 * fixa.
 */
export function ThemePicker({
  themes,
  state,
  mode,
  effectiveBase,
  fontSize,
  footer,
  onImport,
  onCopyExample,
}: {
  themes: Theme[];
  state: ThemeState | null;
  mode: ThemeMode;
  effectiveBase: Base;
  fontSize: number;
  footer: boolean;
  onImport: () => void;
  onCopyExample: () => void;
}) {
  const { t } = useTranslation();
  const slots: Base[] = mode === "system" ? ["light", "dark"] : [mode];
  const [picked, setPicked] = useState<Base>(effectiveBase);
  useEffect(() => setPicked(effectiveBase), [effectiveBase]);
  const slot: Base = slots.includes(picked) ? picked : slots[0];
  const [peek, setPeek] = useState<Theme | null>(null);
  // Sair da tela no meio de uma pré-visualização não pode deixar o app pintado.
  useEffect(() => () => previewTheme(null), []);

  const unpeek = () => {
    setPeek(null);
    previewTheme(null);
  };
  const chosen = state ? state[slot].id : null;
  const shown = themes.filter((theme) => theme.base === slot);
  const slotLabel = (base: Base) => (base === "light" ? t("themeLight") : t("themeDark"));
  const note =
    mode === "system"
      ? t("cfgThemeNoteSystem", { os: IS_MAC ? "macOS" : t("cfgTheSystem") })
      : mode === "light"
        ? t("cfgThemeNoteLight")
        : t("cfgThemeNoteDark");

  return (
    <>
      <div className={`mt-[26px] grid gap-3.5 ${slots.length > 1 ? "grid-cols-2" : "grid-cols-1"}`}>
        {state &&
          slots.map((base) => {
            const theme = base === slot && peek ? peek : state[base];
            return (
              <TerminalSpecimen
                key={base}
                theme={theme}
                label={slotLabel(base)}
                fontSize={fontSize}
                footer={footer}
                selectable={slots.length > 1}
                selected={base === slot}
                onPick={() => {
                  setPicked(base);
                  unpeek();
                }}
              />
            );
          })}
      </div>
      <div className="mt-[18px] flex flex-col gap-3">
        <div className="flex flex-wrap gap-1.5" onMouseLeave={unpeek}>
          {shown.map((theme) => {
            const on = theme.id === chosen;
            return (
              <button
                key={theme.id}
                type="button"
                aria-pressed={on}
                onMouseEnter={() => {
                  setPeek(theme);
                  previewTheme(theme);
                }}
                onFocus={() => {
                  setPeek(theme);
                  previewTheme(theme);
                }}
                onBlur={unpeek}
                onClick={() => {
                  setPeek(null);
                  void setSlotTheme(theme);
                }}
                className={`inline-flex h-9 items-center gap-2 rounded-full pr-3.5 pl-2 text-[13px] transition-colors focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)] ${
                  on
                    ? "bg-tyba-blue-tint font-semibold text-tyba-blue"
                    : "text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-[var(--tyba-glass-piece)]"
                }`}
              >
                <span
                  aria-hidden
                  className="grid size-[22px] place-items-center rounded-full"
                  style={{
                    background: theme.terminal.background,
                    boxShadow: `inset 0 0 0 1px ${lineOf(theme)}`,
                  }}
                >
                  <i className="block size-2 rounded-full" style={{ background: accentOf(theme) }} />
                </span>
                {theme.name}
              </button>
            );
          })}
          <button
            type="button"
            onClick={onImport}
            className="inline-flex h-9 items-center gap-2 rounded-full pr-3.5 pl-2 text-[13px] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-[var(--tyba-glass-piece)] focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)]"
          >
            <Plus size={14} />
            {t("importTheme")}
          </button>
        </div>
        <div className="text-[12.5px] text-tyba-text-faint">
          {note}{" "}
          <button
            type="button"
            onClick={onCopyExample}
            className="font-semibold text-tyba-blue hover:underline"
          >
            {t("cfgThemeCopyExample")}
          </button>
        </div>
      </div>
    </>
  );
}

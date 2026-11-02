import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  Browser,
  Code,
  Cube,
  FolderSimple,
  NotePencil,
  PlugsConnected,
  Terminal,
  TerminalWindow,
} from "@phosphor-icons/react";

import { ClaudeIcon } from "@/components/icons/ClaudeIcon";
import { OpenAIIcon } from "@/components/icons/OpenAIIcon";
import {
  filterPalette,
  type PaletteEntry,
  type PaletteIcon,
  type PaletteLink,
} from "@/lib/graphPalette";
import { IS_MAC } from "@/lib/platform";

const ICON: Record<PaletteIcon, ReactNode> = {
  claude: <ClaudeIcon size={13} />,
  codex: <OpenAIIcon size={13} />,
  process: <TerminalWindow size={13} />,
  terminal: <Terminal size={13} />,
  service: <Cube size={13} />,
  browser: <Browser size={13} />,
  note: <NotePencil size={13} />,
  integration: <PlugsConnected size={13} />,
  snippet: <Code size={13} />,
  folder: <FolderSimple size={13} />,
};

export const PALETTE_W = 320;
export const PALETTE_H = 300;

/**
 * A paleta de nós (quadro OrbitCanvasLiso), aberta onde se pediu: busca,
 * a linha "aceita url · de code-platform" quando veio de uma ligação solta no
 * vazio, a lista (↑↓, ↵ cria) e o rodapé com os jeitos de abrir. Esc ou
 * clicar fora fecha.
 */
export function NodePalette({
  at,
  entries,
  link,
  linkLine,
  onPick,
  onClose,
}: {
  /** Canto da paleta, em px dentro do canvas. */
  at: { x: number; y: number };
  entries: readonly PaletteEntry[];
  link: PaletteLink | null;
  /** "aceita url · de code-platform", quando veio de uma ligação. */
  linkLine: string | null;
  onPick: (entry: PaletteEntry) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useMemo(() => filterPalette(entries, query, link), [entries, query, link]);
  const current = Math.min(active, Math.max(0, list.length - 1));

  useEffect(() => input.current?.focus(), []);
  useEffect(() => setActive(0), [query]);

  const mod = IS_MAC ? "⌘" : "Ctrl+";
  return (
    <>
      <div aria-hidden onMouseDown={onClose} className="absolute inset-0 z-20" />
      <div
        data-occludes
        className="absolute z-30 flex flex-col rounded-[10px] border border-tyba-border-strong bg-tyba-raised font-mono text-[12px] shadow-[var(--tyba-shadow-lg)]"
        style={{ left: at.x, top: at.y, width: PALETTE_W, height: PALETTE_H }}
      >
        <div className="border-b border-tyba-border px-3 py-2.5">
          <input
            ref={input}
            aria-label={t("orbitAddNode")}
            placeholder={t("graphPaletteSearch")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              } else if (e.key === "ArrowDown") {
                e.preventDefault();
                setActive((current + 1) % Math.max(1, list.length));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setActive((current - 1 + list.length) % Math.max(1, list.length));
              } else if (e.key === "Enter") {
                e.preventDefault();
                const entry = list[current];
                if (entry) onPick(entry);
              }
            }}
            className="w-full rounded-[6px] border border-tyba-violet/60 bg-tyba-sunken px-2.5 py-1.5 text-tyba-text outline-none placeholder:text-tyba-text-ghost"
          />
        </div>
        {linkLine && <div className="px-3 pt-1.5 pb-0.5 text-tyba-text-faint">{linkLine}</div>}
        <div
          role="listbox"
          aria-label={t("orbitAddNode")}
          className="flex min-h-0 flex-col gap-0.5 overflow-y-auto px-1.5 py-1"
        >
          {list.map((entry, i) => (
            <button
              key={entry.id}
              type="button"
              role="option"
              aria-selected={i === current}
              onMouseEnter={() => setActive(i)}
              onClick={() => onPick(entry)}
              className={`flex min-w-0 items-center gap-2 rounded-[6px] px-2 py-[7px] text-left ${
                i === current ? "bg-tyba-violet/15 text-tyba-text" : "text-tyba-text-muted"
              }`}
            >
              <span className="flex shrink-0">{ICON[entry.icon]}</span>
              <span className="shrink-0 truncate">{entry.label}</span>
              <span className="min-w-0 flex-1 truncate text-tyba-text-faint">{entry.detail}</span>
              {i === current && <span className="shrink-0 text-tyba-text-faint">↵</span>}
            </button>
          ))}
          {list.length === 0 && (
            <div className="px-2 py-[7px] text-tyba-text-faint">{t("graphPaletteEmpty")}</div>
          )}
          {query.trim() === "" && (
            <div className="px-2 py-1 text-tyba-text-faint">{t("graphPaletteMore")}</div>
          )}
        </div>
        <div className="mt-auto border-t border-tyba-border px-3 py-2 text-tyba-text-faint">
          {t("graphPaletteFooter", { mod })}
        </div>
      </div>
    </>
  );
}

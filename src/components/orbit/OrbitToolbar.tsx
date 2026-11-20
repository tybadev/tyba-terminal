import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ChatCircle, Code, Cursor, Globe, Hand, LinkSimple, Note, Play, Sparkle } from "@phosphor-icons/react";

import { GLASS_PILL } from "./drawer";
import type { OrbitTool, PointerTool } from "./toolbar";

const TOOLS: (readonly [OrbitTool, string, ReactNode] | "|")[] = [
  ["cursor", "orbitToolCursor", <Cursor size={20} />],
  ["hand", "orbitToolHand", <Hand size={20} />],
  "|",
  ["agent", "orbitToolAgent", <Sparkle size={20} />],
  ["term", "orbitToolTerminal", <Code size={20} />],
  ["proc", "orbitToolProcess", <Play size={20} />],
  ["note", "orbitToolNote", <Note size={20} />],
  ["web", "orbitToolBrowser", <Globe size={20} />],
  "|",
  ["link", "orbitToolLink", <LinkSimple size={20} />],
];

/**
 * As ferramentas do Orbit (quadro Orbit): a barra do Muriki no pé do canvas,
 * com o brilho azul e amarelo atrás. Selecionar e Mover trocam o modo do
 * ponteiro (aceso em azul); os outros criam o nó no centro da vista, e Ligar
 * liga os dois nós selecionados.
 */
export function OrbitToolbar({
  pointer,
  canLink,
  onTool,
  talk,
  onTalk,
}: {
  pointer: PointerTool;
  canLink: boolean;
  onTool: (tool: OrbitTool) => void;
  /** O orquestrador vivo para "Falar com o Orquestrador"; `null`, apagado. */
  talk: { waiting: boolean } | null;
  onTalk: () => void;
}) {
  const { t } = useTranslation();
  return (
    <>
      {/* Só no claro do TYBA (Muriki): o escuro não ganha cor no campo. */}
      <div
        aria-hidden
        className="pointer-events-none absolute -bottom-[60px] left-1/2 z-[4] hidden h-[200px] w-[760px] -translate-x-1/2 blur-[10px] [[data-theme='light']:not([data-imported])_&]:block"
        style={{
          background:
            "radial-gradient(40% 60% at 25% 50%, color-mix(in srgb, var(--tyba-primary) 35%, transparent), transparent 70%), radial-gradient(35% 55% at 82% 45%, color-mix(in srgb, var(--tyba-amber-fill) 45%, transparent), transparent 70%)",
        }}
      />
      <div
        role="toolbar"
        aria-label={t("orbitTools")}
        className={`absolute bottom-5 left-1/2 z-[5] inline-flex -translate-x-1/2 items-center gap-0.5 p-1.5 ${GLASS_PILL}`}
      >
        {TOOLS.map((tool, i) => {
          if (tool === "|") {
            return (
              <span key={i} aria-hidden className="mx-1.5 h-[22px] w-px bg-[var(--tyba-line)]" />
            );
          }
          const [id, label, icon] = tool;
          const on = id === pointer;
          const disabled = id === "link" && !canLink;
          return (
            <button
              key={id}
              type="button"
              aria-label={t(label)}
              title={disabled ? t("orbitToolLinkHint") : t(label)}
              aria-pressed={id === "cursor" || id === "hand" ? on : undefined}
              disabled={disabled}
              onClick={() => onTool(id)}
              className={`grid size-[38px] place-items-center rounded-full transition-[background-color,transform] duration-150 active:scale-[.96] disabled:cursor-not-allowed disabled:opacity-45 ${
                on
                  ? "bg-[var(--tyba-primary)] text-tyba-text-inverse shadow-[var(--tyba-shadow-sm)]"
                  : "text-tyba-text-muted enabled:hover:bg-tyba-text/[.06] enabled:hover:text-tyba-text"
              }`}
              >
              {icon}
              </button>
          );
        })}
        {/* Quadro Orbit: o balão no fim da barra, com o ponto amarelo quando
            o orquestrador espera você. Abre ele no modo foco. */}
        <button
          type="button"
          aria-label={t("orbitToolTalk")}
          title={talk ? t("orbitToolTalk") : t("orbitToolTalkHint")}
          disabled={!talk}
          onClick={onTalk}
          className="relative grid size-[38px] place-items-center rounded-full text-tyba-text-muted transition-[background-color,transform] duration-150 active:scale-[.96] enabled:hover:bg-tyba-text/[.06] enabled:hover:text-tyba-text disabled:cursor-not-allowed disabled:opacity-45"
        >
          <ChatCircle size={20} />
          {talk?.waiting && (
            <span
              aria-hidden
              className="absolute right-[7px] top-[7px] size-2 rounded-full bg-tyba-amber-fill shadow-[0_0_0_2px_var(--tyba-paper-solid,#fff)]"
            />
          )}
        </button>
      </div>
    </>
  );
}

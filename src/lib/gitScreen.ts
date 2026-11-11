// Tela cheia de Git (protótipo "Git em tela cheia"): desliza da direita por
// cima das abas e do shell quando o painel lateral do workspace é `git:<id>`.
// Quem guarda se ela está aberta é o core (`side_view`); daqui sai só qual
// seção mostrar — estado de tela, efêmero, como o modo do diff.

import { useSyncExternalStore } from "react";

import { openGitPanel, type Session, type SessionId } from "./ipc";

export type GitSection = "changes" | "turn" | "review" | "history" | "prs" | "ci";

/** Ordem do trilho, de cima para baixo. */
export const GIT_SECTIONS: readonly GitSection[] = [
  "changes",
  "turn",
  "review",
  "history",
  "prs",
  "ci",
];

export type GitPlace = {
  section: GitSection;
  pr: number | null;
  /** Arquivo a abrir na seção Turno (clique na linha da aba Turno). */
  path?: string | null;
};

const DEFAULT: GitPlace = { section: "changes", pr: null };

/**
 * O último turno da sessão tem diff para ver: fechou medido e mexeu em
 * arquivo. É quando existem a seção Turno e o "Ver diff do turno".
 */
export function hasTurnDiff(session: Pick<Session, "last_turn"> | null | undefined): boolean {
  const turn = session?.last_turn;
  return Boolean(turn && turn.diff === "ok" && turn.files_total > 0);
}

/**
 * Seções do trilho para a sessão: Revisar (diff contra a base congelada) só
 * existe em worktree; Turno, enquanto o último turno tiver diff.
 */
export function railSections(hasWorktree: boolean, hasTurn = false): GitSection[] {
  return GIT_SECTIONS.filter(
    (s) => (s !== "review" || hasWorktree) && (s !== "turn" || hasTurn),
  );
}

/** Para onde o conteúdo novo desliza: descer no trilho entra de baixo. */
export function slideOf(
  from: GitPlace,
  to: GitPlace,
): "up" | "down" | "left" | "right" {
  if (from.section === to.section) {
    if (to.pr !== null && from.pr === null) return "left";
    if (to.pr === null && from.pr !== null) return "right";
    return "up";
  }
  return GIT_SECTIONS.indexOf(to.section) >= GIT_SECTIONS.indexOf(from.section) ? "up" : "down";
}

/** Seção pedida que a sessão não tem cai em Alterações. */
export function resolvePlace(place: GitPlace, hasWorktree: boolean, hasTurn = false): GitPlace {
  return railSections(hasWorktree, hasTurn).includes(place.section) ? place : DEFAULT;
}

// --- onde cada sessão estava, com assinatura -------------------------------

const places = new Map<SessionId, GitPlace>();
let version = 0;
const listeners = new Set<() => void>();

function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

export function gitPlaceOf(session: SessionId): GitPlace {
  return places.get(session) ?? DEFAULT;
}

export function setGitPlace(session: SessionId, place: GitPlace): void {
  places.set(session, place);
  version += 1;
  for (const fn of listeners) fn();
}

export function useGitPlace(session: SessionId): GitPlace {
  useSyncExternalStore(subscribe, () => version);
  return gitPlaceOf(session);
}

/** Abre a tela de Git da sessão já na seção pedida. */
export function openGitScreen(
  session: SessionId,
  section: GitSection,
  pr: number | null = null,
  path: string | null = null,
): Promise<void> {
  setGitPlace(session, { section, pr, path });
  return openGitPanel(session);
}

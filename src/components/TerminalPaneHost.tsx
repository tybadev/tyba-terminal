import { createContext, useContext, useEffect, type ReactNode } from "react";

import type { SessionId } from "../lib/ipc";

/**
 * O painel da aba Terminal (blocos, a linha do TYBA, Rich Input) emprestado a
 * quem está fora da grade — o nó Terminal do Orbit.
 *
 * O estado do painel é todo por sessão e mora no `App` (comando, modo prompt,
 * cwd, blocos, rascunho…). Em vez de copiar esse estado para o Orbit, o `App`
 * entrega o mesmo `renderPane` da grade por este contexto: o nó só diz qual
 * sessão e em que zoom.
 */
export interface TerminalPaneHost {
  /**
   * O painel da sessão. `zoom` é o do canvas (o xterm fora da escala, ver
   * `nodeTerminal`); ausente no modo foco, em tamanho real. `null` enquanto
   * o `App` ainda não conhece a sessão (ver `attach`).
   */
  render: (sessionId: SessionId, zoom?: number) => ReactNode;
  /**
   * Pede ao `App` o estado de painel desta sessão (a linha, a lista de
   * sessões). Devolve o soltar.
   */
  attach: (sessionId: SessionId) => () => void;
  /** Enter no nó: a linha do TYBA quando ela é dona do teclado, senão o xterm. */
  focusInput: (sessionId: SessionId) => void;
}

export const TerminalPaneContext = createContext<TerminalPaneHost | null>(null);

export function useTerminalPane(): TerminalPaneHost | null {
  return useContext(TerminalPaneContext);
}

/** O painel da aba Terminal para uma sessão de fora da grade. */
export function HostedTerminalPane({
  sessionId,
  zoom,
}: {
  sessionId: SessionId;
  zoom?: number;
}) {
  const host = useTerminalPane();
  const attach = host?.attach;
  useEffect(() => attach?.(sessionId), [attach, sessionId]);
  return <>{host?.render(sessionId, zoom) ?? null}</>;
}

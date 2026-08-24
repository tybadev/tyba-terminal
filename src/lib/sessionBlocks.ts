import type { Block, PaneId, SessionCwd, SessionId } from "./ipc";

/**
 * A lista vazia, com identidade fixa.
 *
 * `blocks[id] ?? []` parece inofensivo e é exatamente o que derruba a
 * comparação rasa: o literal nasce de novo a cada render do `App`, e a sessão
 * que ainda não tem bloco nenhum re-renderiza a lista inteira por causa dele.
 */
const NO_BLOCKS: Block[] = [];

/**
 * As props de dados de um painel de blocos — tudo primitivo, ou referência que
 * já vinha estável do `App`.
 *
 * > [!warning] Nenhum campo aqui pode ser objeto ou função construídos na hora.
 * > `SessionBlocks` é `memo`, e memo compara props com `Object.is` campo a
 * > campo: um `{ cwd, atMs }` montado a cada chamada faz a comparação falhar
 * > sempre, e a memoização vira custo puro. Retângulo e `opened` são montados
 * > DENTRO do componente, a partir dos números daqui.
 */
export interface SessionBlocksData {
  sessionId: SessionId;
  paneId: PaneId;
  left: number;
  top: number;
  width: number;
  height: number;
  blocks: Block[];
  live: boolean;
  used: number;
  headerPx: number;
  fontSizePx: number;
  lineHeightPx: number;
  cellWidthPx: number;
  openedCwd: string | null;
  openedAtMs: number | null;
  active: boolean;
  command: string;
  marked: ReadonlySet<number> | undefined;
  copyCombo: string;
}

/**
 * Recorta o estado do `App` no que o painel de blocos de UMA sessão consome.
 *
 * Existe separada do componente para poder ser conferida por teste: o defeito
 * que ela previne compila, roda e não aparece na tela — só custa quadro.
 */
export function sessionBlocksData(input: {
  session: { id: SessionId; created_at: string };
  pane: { pane: PaneId; x: number; y: number; w: number; h: number };
  blocks: Block[] | undefined;
  live: boolean;
  /** Quanto da faixa ao vivo a saída ocupa. Ausente = faixa cheia. */
  used: number | undefined;
  /** Altura medida do header do bloco em execução. Ausente = ainda não mediu. */
  headerPx: number | undefined;
  fontSizePx: number;
  lineHeightPx: number;
  cellWidthPx: number;
  cwd: SessionCwd | undefined;
  active: boolean;
  command: string | null | undefined;
  marked: ReadonlySet<number> | null;
  copyCombo: string;
}): SessionBlocksData {
  return {
    sessionId: input.session.id,
    paneId: input.pane.pane,
    left: input.pane.x,
    top: input.pane.y,
    width: input.pane.w,
    height: input.pane.h,
    blocks: input.blocks ?? NO_BLOCKS,
    live: input.live,
    used: input.used ?? 1,
    headerPx: input.headerPx ?? 0,
    fontSizePx: input.fontSizePx,
    lineHeightPx: input.lineHeightPx,
    cellWidthPx: input.cellWidthPx,
    openedCwd: input.cwd?.cwd ?? input.cwd?.canonical ?? null,
    openedAtMs: Date.parse(input.session.created_at) || null,
    active: input.active,
    command: input.command ?? "",
    marked: input.marked ?? undefined,
    copyCombo: input.copyCombo,
  };
}

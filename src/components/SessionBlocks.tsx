import { memo, useCallback, useMemo } from "react";

import {
  ActiveBlockFrame,
  ActiveBlockHeader,
  blocksRect,
  liveRect,
  padSlackPx,
} from "./ActiveBlock";
import { BLOCK_GAP_PX, BlockList } from "./BlockList";
import { LIVE_PAD_Y_PX } from "./TerminalView";
import type { PaneId, SessionId } from "../lib/ipc";
import type { SessionBlocksData } from "../lib/sessionBlocks";

interface Props extends SessionBlocksData {
  /** Devolve o comando para a linha. Só o painel ativo tem para onde injetar. */
  onInject: (text: string) => void;
  /**
   * O `focusPane` do core, cru.
   *
   * Cru de propósito: um `() => focusPane(pane)` montado pelo `App` dentro do
   * `map` nasce com identidade nova a cada render e derruba o `memo` deste
   * componente. Quem amarra o painel é o `useCallback` aqui dentro, onde a
   * dependência é um `PaneId` — comparável por valor.
   */
  onFocusPane: (pane: PaneId) => Promise<void>;
  onPick: (id: number, event: React.MouseEvent) => void;
  onClearPick: () => void;
  onHeaderPx: (id: SessionId, px: number) => void;
}

/**
 * O painel de blocos de uma sessão: a lista, o header do comando em execução e
 * a moldura da faixa ao vivo.
 *
 * Existe para dar uma FRONTEIRA de memoização por sessão. O `App` é um
 * componente só e re-renderiza a ~60 Hz enquanto um comando escreve; sem esta
 * fronteira, o corpo deste painel era montado inline dentro de um `map` e todo
 * `rect`, `opened` e `onActivate` nascia de novo em cada quadro — o `memo` da
 * `BlockList` comparava props sempre diferentes e nunca curto-circuitava, em
 * TODO painel, inclusive nos que nada tinham a ver com o comando rodando.
 *
 * > [!warning] As props têm de continuar sendo primitivos e referências
 * > estáveis. Objeto ou arrow montados no JSX do `App` recolocam o defeito, e
 * > ele não aparece na tela: compila, roda e só custa quadro. Ver
 * > `lib/sessionBlocks` e o teste dele.
 */
export const SessionBlocks = memo(function SessionBlocks({
  sessionId,
  paneId,
  left,
  top,
  width,
  height,
  blocks,
  live,
  used,
  headerPx,
  fontSizePx,
  lineHeightPx,
  cellWidthPx,
  openedCwd,
  openedAtMs,
  active,
  command,
  marked,
  copyCombo,
  onInject,
  onFocusPane,
  onPick,
  onClearPick,
  onHeaderPx,
}: Props) {
  const pane = useMemo(
    () => ({ left, top, width, height }),
    [left, top, width, height],
  );
  const listRect = useMemo(
    () => blocksRect(pane, live, used),
    [pane, live, used],
  );
  const liveBox = useMemo(() => liveRect(pane, used), [pane, used]);
  // A saída sobe além do que a conta em % diz, porque o recorte desconta o
  // padding do terminal. Lista, header e moldura acompanham pelo mesmo tanto.
  const lift = padSlackPx(LIVE_PAD_Y_PX, used);
  const opened = useMemo(
    () => ({ cwd: openedCwd, atMs: openedAtMs }),
    [openedCwd, openedAtMs],
  );
  const activate = useCallback(() => {
    void onFocusPane(paneId);
  }, [onFocusPane, paneId]);
  const reportHeaderPx = useCallback(
    (px: number) => onHeaderPx(sessionId, px),
    [onHeaderPx, sessionId],
  );

  return (
    <>
      {/* Sem `blocks.length > 0`: a lista é o que COBRE o terminal, e o
          terminal em modo prompt é meia altura do painel. Escondida enquanto
          não houvesse bloco, o painel recém-aberto mostrava a caixa do xterm no
          rodapé e vazio em cima — o "abre já menor" do split. Vazia ela é um
          scroller com o cartão-zero dentro. */}
      <BlockList
        blocks={blocks}
        rect={listRect}
        bottomInset={live ? headerPx + lift + BLOCK_GAP_PX : 0}
        fontSizePx={fontSizePx}
        lineHeightPx={lineHeightPx}
        cellWidthPx={cellWidthPx}
        opened={opened}
        onInject={active ? onInject : undefined}
        onActivate={active ? undefined : activate}
        marked={marked}
        onPick={onPick}
        onClearPick={onClearPick}
        copyCombo={copyCombo}
      />
      {live && (
        <>
          <ActiveBlockHeader
            command={command}
            rect={liveBox}
            liftPx={lift}
            onHeight={reportHeaderPx}
          />
          <ActiveBlockFrame rect={liveBox} liftPx={lift} />
        </>
      )}
    </>
  );
});

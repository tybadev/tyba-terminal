import { memo, useCallback, useEffect, useReducer, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  ArrowUUpLeft,
  Check,
  ClipboardText,
  Copy,
  MarkdownLogo,
} from "@phosphor-icons/react";

import { bodyLines, columnsFor } from "../lib/blockMetrics";
import {
  blockMarkdown,
  blockOutput,
  duration,
  failed,
  shortPath,
} from "../lib/blockText";
import { writeClipboardText } from "../lib/clipboard";
import type { Block, BlockColor, LogicalLine, StyleRun } from "../lib/ipc";
import { toastError } from "../lib/toast";
import { ansiColor, onTerminalThemeChange } from "../theme";
import type { PaneRectStyle } from "./TerminalView";

/**
 * Cor indexada é resolvida na RENDERIZAÇÃO, não na captura: o bloco guarda o
 * índice para acompanhar troca de tema. `Default` devolve `undefined` e herda a
 * cor do container.
 */
function cssColor(color: BlockColor): string | undefined {
  if (color.kind === "rgb") {
    const [r, g, b] = color.value;
    return `rgb(${r} ${g} ${b})`;
  }
  if (color.kind === "idx") return ansiColor(color.value);
  return undefined;
}

function runStyle(run: StyleRun): React.CSSProperties {
  return {
    color: cssColor(run.fg),
    backgroundColor: cssColor(run.bg),
    fontWeight: run.bold ? 600 : undefined,
    fontStyle: run.italic ? "italic" : undefined,
    textDecoration: run.underline ? "underline" : undefined,
  };
}

/**
 * Uma linha lógica vira spans. Nunca HTML: os runs são dados, e montar string
 * de markup a partir de saída de comando é injeção esperando acontecer.
 *
 * Memoizado porque este é o nó que se multiplica: uma linha com cor gera até
 * dois spans por run, e saída colorida dá uns 16 runs por linha. Um cartão de
 * 200 linhas passa de 3 mil nós, e sem memo TODOS eram recriados a cada render
 * do `App` — que acontece a ~60 Hz enquanto um comando escreve.
 *
 * O objeto `line` vem do bloco e mantém identidade: `setBlocks` recria o array,
 * não os blocos. É o que faz a comparação rasa do memo valer aqui.
 */
const Line = memo(function Line({ line }: { line: LogicalLine }) {
  if (line.runs.length === 0) {
    return <div className="whitespace-pre-wrap break-words">{line.text}</div>;
  }
  const parts: React.ReactNode[] = [];
  let cursor = 0;
  line.runs.forEach((run, i) => {
    if (run.start > cursor) {
      parts.push(<span key={`p${i}`}>{line.text.slice(cursor, run.start)}</span>);
    }
    parts.push(
      <span key={`r${i}`} style={runStyle(run)}>
        {line.text.slice(run.start, run.end)}
      </span>,
    );
    cursor = run.end;
  });
  if (cursor < line.text.length) {
    parts.push(<span key="tail">{line.text.slice(cursor)}</span>);
  }
  return <div className="whitespace-pre-wrap break-words">{parts}</div>;
});

/** Quanto tempo o ✓ fica no lugar do ícone depois de copiar. */
const COPIED_MS = 1200;

type ActionId = "command" | "output" | "markdown";

/**
 * Copiar comando, copiar saída, copiar como markdown, devolver para a linha.
 *
 * Tudo sai do modelo do bloco — ver `lib/blockText`. Ler do DOM entregaria o
 * corpo cortado em `BODY_LIMIT` linhas, e a saída sem os espaços que o
 * `white-space` desenha mas o `textContent` não tem.
 */
const BlockActions = memo(function BlockActions({
  block,
  onInject,
}: {
  block: Block;
  onInject?: (text: string) => void;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState<ActionId | null>(null);
  const timer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const copy = useCallback(
    (id: ActionId, text: string) => {
      void writeClipboardText(text)
        .then(() => {
          setCopied(id);
          if (timer.current !== null) window.clearTimeout(timer.current);
          timer.current = window.setTimeout(() => setCopied(null), COPIED_MS);
        })
        .catch((error) => toastError(t("blockCopyFailed"), error));
    },
    [t],
  );

  const hasOutput = block.lines.length > 0;
  const items: Array<{
    id: ActionId | "inject";
    label: string;
    icon: typeof Copy;
    run: () => void;
    show: boolean;
  }> = [
    {
      id: "command",
      label: t("blockCopyCommand"),
      icon: Copy,
      run: () => copy("command", block.command),
      show: block.command.length > 0,
    },
    {
      id: "output",
      label: t("blockCopyOutput"),
      icon: ClipboardText,
      run: () => copy("output", blockOutput(block)),
      show: hasOutput,
    },
    {
      id: "markdown",
      label: t("blockCopyMarkdown"),
      icon: MarkdownLogo,
      run: () => copy("markdown", blockMarkdown(block)),
      show: block.command.length > 0 || hasOutput,
    },
    {
      id: "inject",
      label: t("blockReuse"),
      icon: ArrowUUpLeft,
      // Reinsere na linha e para por aí: quem aperta Enter é o dono.
      run: () => onInject?.(block.command),
      show: Boolean(onInject) && block.command.length > 0,
    },
  ];

  return (
    <div
      // Sempre presentes, e discretos — não escondidos atrás do hover.
      //
      // Antes o cartão normal nascia em `opacity-0` e só o header PRESO ficava
      // opaco, porque ele é `pointer-events-none` e nunca recebe hover. O
      // resultado é que o mesmo bloco mostrava quatro ícones no topo da lista e
      // nenhum logo abaixo: a barra de ações parecia ir e vir sem regra.
      //
      // Visível de saída também responde à pergunta "dá para copiar isso?" sem
      // exigir que se descubra passando o mouse. A 40% ela informa sem competir
      // com o comando, que é o que a linha tem de mais importante.
      className="pointer-events-auto flex shrink-0 items-center gap-0.5 opacity-40 transition-opacity focus-within:opacity-100 group-hover:opacity-100"
    >
      {items
        .filter((item) => item.show)
        .map((item) => {
          const Icon = copied === item.id ? Check : item.icon;
          return (
            <button
              key={item.id}
              type="button"
              title={item.label}
              aria-label={item.label}
              // Clicar numa ação não é clicar no bloco: sem isto, copiar a
              // saída marcaria o cartão de quebra.
              onClick={(event) => {
                event.stopPropagation();
                item.run();
              }}
              // Caixa de tamanho fixo, e menor que a linha do comando: botão
              // que cresce o header desalinha a estimativa de altura do
              // virtualizador e o bloco pula quando a medição real chega.
              className="flex size-[18px] items-center justify-center rounded-[3px] text-tyba-text-faint transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text focus-visible:bg-tyba-text/[.06] focus-visible:text-tyba-text"
            >
              <Icon size={13} weight={copied === item.id ? "bold" : "regular"} />
            </button>
          );
        })}
    </div>
  );
});

const BlockHeader = memo(function BlockHeader({
  block,
  pinned,
  onInject,
}: {
  block: Block;
  pinned?: boolean;
  onInject?: (text: string) => void;
}) {
  const broke = failed(block.exitCode);
  const took = duration(block);
  const where = shortPath(block.cwd);
  return (
    <div
      // O preso é `pointer-events-none`: ele fica POR CIMA da lista, e uma
      // faixa opaca de 27px que captura a roda do mouse é uma faixa onde a
      // lista não rola. Só as ações reativam — senão as do bloco do topo, que
      // é o header que ele cobre, ficariam inalcançáveis.
      // O fundo e o filete do preso moram no INVÓLUCRO, não aqui: só assim ele
      // atravessa a largura inteira da lista enquanto o texto continua alinhado
      // com o comando do cartão de baixo, que é recuado pelo padding do
      // scroller. Com fundo próprio, a faixa era um cartão solto de 8px de
      // margem e o comando dela caía 8px à esquerda do comando de todos os
      // outros — ficava lendo como outra coisa, não como o mesmo header.
      className={`group flex items-center gap-2 px-2.5 py-1 ${
        pinned ? "pointer-events-none" : "border-b border-tyba-border/60"
      }`}
    >
      <span className={`shrink-0 ${broke ? "text-tyba-red" : "text-tyba-green"}`}>
        ❯
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-tyba-text">
        {block.command}
      </span>
      {/* Onde rodou. Encurtado porque o começo do caminho empurraria o comando
          para fora da linha; o inteiro fica no title. */}
      {where && (
        <span
          title={block.cwd ?? undefined}
          className="max-w-[38%] shrink-0 truncate font-mono text-[10px] text-tyba-text-faint"
        >
          {where}
        </span>
      )}
      {broke && (
        <span className="shrink-0 font-mono text-[10px] text-tyba-red">
          {block.exitCode}
        </span>
      )}
      {took && (
        <span className="shrink-0 font-mono text-[10px] tabular-nums text-tyba-text-faint">
          {took}
        </span>
      )}
      <BlockActions block={block} onInject={onInject} />
    </div>
  );
});

/**
 * Teto de linhas desenhadas de uma vez.
 *
 * O virtualizador virtualiza BLOCOS, não as linhas dentro de um: um bloco no
 * teto de 10 mil linhas viraria 10 mil nós de DOM num item só e travaria o
 * painel. Ninguém lê 10 mil linhas rolando — quem precisa, expande.
 */
const BODY_LIMIT = 200;

/**
 * Altura do botão "mostrar mais" (`text-[10px]` + `py-1` + `border-t`).
 *
 * Entra na estimativa. Sem ele, TODO bloco grande era estimado ~22px curto — e
 * estimativa curta é cartão nascendo por cima do fim do anterior.
 */
const SHOW_MORE_PX = 22;

/**
 * Quanto da largura do scroller NÃO é texto do corpo, em px.
 *
 * `px-2` do scroller (8+8), a borda do cartão (1+1) e o `px-2.5` do corpo
 * (10+10). Entra na conta de quantas colunas cabem numa linha — errar aqui
 * volta como estimativa curta, que é cartão em cima de cartão.
 */
const BODY_INSET_X_PX = 38;

/**
 * Barra à esquerda do cartão marcado.
 *
 * `box-shadow` e não borda: borda muda a altura medida, e altura que muda faz o
 * virtualizador reposicionar a lista embaixo do ponteiro no meio de um
 * shift-clique.
 */
/**
 * A barra na lateral do bloco marcado.
 *
 * `box-shadow` e não borda: borda muda a altura medida, e altura que muda faz o
 * virtualizador reposicionar a lista embaixo do ponteiro no meio de um
 * shift-clique.
 *
 * 2px, e não mais: com a borda do marcado já na cor de destaque, engrossar a
 * barra faz a lateral esquerda destoar do resto do contorno — 6px contra 2px
 * do topo lê como caixa torta. Mais destaque se pede à cor, não à espessura.
 */
const MARKED_BAR = "inset 2px 0 0 0 var(--tyba-primary)";


const BlockCard = memo(function BlockCard({
  block,
  fontSizePx,
  lineHeightPx,
  onInject,
  marked,
  onPick,
  shown,
  onShowMore,
}: {
  block: Block;
  /** Tamanho da fonte do terminal — ver o corpo abaixo. */
  fontSizePx: number;
  /** Altura real de uma linha do terminal, medida. */
  lineHeightPx: number;
  onInject?: (text: string) => void;
  marked: boolean;
  /**
   * Recebe o id, e não um handler já amarrado ao bloco.
   *
   * Amarrar do lado de fora (`(event) => onPick(block.id, event)`) cria uma
   * função nova por render do pai, e prop com identidade nova derruba o memo
   * deste componente — que é justo o que ele existe para evitar. O id já está
   * no `block`; quem amarra é o corpo daqui, que só roda quando o cartão
   * realmente re-renderiza.
   */
  onPick?: (id: number, event: React.MouseEvent) => void;
  /**
   * Quantas linhas do corpo desenhar. Vem de FORA por dois motivos.
   *
   * Morava aqui, num `useState` booleano de "expandido". Como o cartão desmonta
   * ao sair da tela pela virtualização, o estado ia junto: o dono expandia,
   * rolava, voltava, e estava fechado de novo.
   *
   * E o virtualizador precisa saber. A estimativa de altura é calculada pela
   * lista, que não enxergava esse booleano — um bloco expandido de 5 mil linhas
   * seguia estimado em 200. O total da lista ficava menor que o conteúdo real,
   * e daí "rolo e nunca chego no fim".
   */
  shown: number;
  onShowMore?: (id: number) => void;
}) {
  const { t } = useTranslation();
  const broke = failed(block.exitCode);
  const hidden = Math.max(block.lines.length - shown, 0);
  return (
    // `group` aqui além do header: revela as ações com o ponteiro em qualquer
    // lugar do cartão, não só na faixa de 27px de cima.
    <div
      onClick={onPick ? (event) => onPick(block.id, event) : undefined}
      // Shift-clique é o gesto do navegador para esticar seleção de texto.
      // Barrar aqui é o que deixa o shift ser do bloco, sem sujar a tela com
      // um trecho de texto realçado que ninguém pediu.
      onMouseDown={(event) => {
        if (event.shiftKey && onPick) event.preventDefault();
      }}
      style={marked ? { boxShadow: MARKED_BAR } : undefined}
      className={`group overflow-hidden rounded-[5px] border ${
        marked
          ? "border-tyba-primary bg-tyba-green-tint"
          : broke
            ? "border-tyba-red/50 bg-tyba-red/[.07]"
            : "border-tyba-block-border"
      }`}
    >
      <BlockHeader block={block} onInject={onInject} />
      {/* Sem corpo, mas com motivo: um cartão vazio e mudo pareceria comando
          que não imprimiu nada. */}
      {block.altScreen && (
        <div className="px-2.5 py-1 font-mono text-[11px] italic text-tyba-text-faint">
          {t("blockAltScreen")}
        </div>
      )}
      {block.lines.length > 0 && (
        // Fonte e entrelinha do TERMINAL, não um tamanho próprio: este corpo é
        // a mesma saída que estava no terminal um instante antes, e o dono pode
        // ter mudado o tamanho da fonte. Preso em 13px, o texto encolhia ao
        // virar cartão.
        <div
          className="px-2.5 py-1 font-mono text-tyba-text-muted"
          style={{
            fontSize: `${fontSizePx}px`,
            lineHeight: `${lineHeightPx}px`,
          }}
        >
          {block.lines.slice(0, shown).map(
            (line, i) => (
              <Line key={i} line={line} />
            ),
          )}
        </div>
      )}
      {hidden > 0 && (
        <button
          onClick={(event) => {
            event.stopPropagation();
            onShowMore?.(block.id);
          }}
          className="w-full border-t border-tyba-border/60 px-2.5 py-1 text-left font-mono text-[10px] text-tyba-text-faint hover:text-tyba-text"
        >
          {t("blockShowAll", { count: Math.min(hidden, BODY_LIMIT) })}
        </button>
      )}
      {block.truncated > 0 && (
        <div className="border-t border-tyba-border/60 px-2.5 py-1 font-mono text-[10px] text-tyba-amber">
          {t("blockTruncated", { count: block.truncated })}
        </div>
      )}
    </div>
  );
});

/**
 * O começo da pilha: onde e quando a sessão abriu.
 *
 * Um painel de shell recém-aberto era um vazio absoluto, e como a lista só
 * existia com pelo menos um bloco, o que aparecia ali era a caixa do xterm — que
 * ocupa meia altura do painel. Daí o "o split abre já menor": o painel novo
 * nunca abriu menor, ele só não tinha nada cobrindo a outra metade.
 *
 * Fica FORA da lista virtualizada, como primeiro nó do conteúdo: assim ele não
 * entra em índice, em seleção nem na contagem de marcados, e não há um bloco
 * falso que a cópia possa alcançar. Rola junto porque está dentro do scroller.
 */
function SessionOpened({
  cwd,
  at,
  fontSizePx,
}: {
  cwd: string | null;
  /** Quando a sessão abriu, em ms. */
  at: number | null;
  fontSizePx: number;
}) {
  const { t } = useTranslation();
  const where = shortPath(cwd);
  const when =
    at === null || !Number.isFinite(at)
      ? null
      : new Date(at).toLocaleTimeString(undefined, {
          hour: "2-digit",
          minute: "2-digit",
        });
  return (
    <div
      // Sem borda cheia: é uma marca de começo, não um bloco. Cartão de verdade
      // aqui competiria com o primeiro comando de fato.
      className="mb-2 flex items-center gap-2 px-2.5 py-1 font-mono text-tyba-text-faint"
      style={{ fontSize: `${Math.max(10, fontSizePx - 2)}px` }}
    >
      <span className="size-1 shrink-0 rounded-full bg-tyba-text-faint/60" />
      <span className="shrink-0">{t("blockSessionOpened")}</span>
      {where && (
        <span title={cwd ?? undefined} className="min-w-0 truncate">
          · {where}
        </span>
      )}
      {when && <span className="shrink-0 tabular-nums">· {when}</span>}
    </div>
  );
}

/** Métricas do cartão, para a estimativa nascer perto do valor medido. */
/// 13px × 1.35, a mesma métrica do xterm.

/**
 * A tampa do cartão: o header MAIS a borda da caixa em volta.
 *
 * 35, e não os 27 de antes: medido no app, um cartão sem corpo dá 43px de altura
 * — 35 de caixa e os 8 do respiro. O número antigo deixava TODA estimativa 8px
 * curta, e estimativa curta é cartão sobrepondo o vizinho enquanto a medida real
 * não chega.
 */
const HEADER_PX = 35;
/**
 * Altura mínima para um bloco prender o header no topo.
 *
 * Três vezes o header: o bloco precisa ter saída o bastante para valer a pena
 * lembrar de quem ela é depois que o header dele sai de cena. Abaixo disso o
 * header real some e volta na mesma rolagem, e prender só acrescenta uma barra
 * piscando — ver a conta na condição de `pinned`.
 */
const PIN_MIN_PX = HEADER_PX * 3;
/**
 * A linha do motivo no cartão de tela cheia (`text-[11px]` + `py-1`).
 *
 * Ele não tem corpo, mas não é só o header: sem esta parcela a estimativa
 * ficava curta para todo `vim`, `htop` ou `less`, e o cartão seguinte nascia
 * por cima dele.
 */
const ALT_SCREEN_PX = 21;
/**
 * Respiro entre um bloco e o seguinte.
 *
 * Exportado porque o bloco EM EXECUÇÃO também precisa dele: sem isso ele é o
 * único da lista encostado no vizinho, e a diferença lê como defeito bem no
 * cartão para onde o olho vai.
 */
export const BLOCK_GAP_PX = 8;
/**
 * Folga para considerar a lista "no fim".
 *
 * Zero exigiria pixel exato: arredondamento de `scrollHeight` fracionário e uma
 * rolagem que parou a um fio do fim deixariam a lista de reancorar justamente
 * quando o dono achava que estava acompanhando a saída.
 */
const STICK_SLACK_PX = 4;

/**
 * Quanto tempo sem rolar até conferir as alturas do que está na tela.
 *
 * Curto o bastante para o dono não ver o desencontro, longo o bastante para não
 * rodar no meio de um gesto de rolagem — que é onde o virtualizador
 * deliberadamente não mede.
 */
const SETTLE_MS = 120;

interface Props {
  blocks: Block[];
  rect: PaneRectStyle;
  /**
   * Devolve o comando para a linha, sem executar. É o mesmo caminho do
   * histórico e do snippet — com a linha do TYBA no ar o terminal está
   * somente-leitura, e `term.paste` seria engolido em silêncio.
   *
   * Ausente quando o painel não é o ativo: a injeção tem um destino só, e
   * mandar o comando para a sessão errada é pior do que não ter o botão.
   */
  onInject?: (text: string) => void;
  /** Mexer nos blocos de um painel torna aquele painel o ativo. */
  onActivate?: () => void;
  /** Ids marcados para copiar de uma vez. */
  marked?: ReadonlySet<number>;
  /** Clique num cartão. O modificador vem no evento; a regra é do chamador. */
  onPick?: (id: number, event: React.MouseEvent) => void;
  /** Clique no fundo da lista — sai da seleção sem procurar onde clicar. */
  onClearPick?: () => void;
  /** Atalho de copiar, já formatado. Vem de fora porque é rebindável. */
  copyCombo?: string;
  /**
   * Altura, em px, do header do bloco em execução — que fica POR CIMA do fim
   * desta lista, não ao lado dela.
   *
   * Ele é desenhado no topo da faixa ao vivo com `translateY(-100%)`, ou seja,
   * ocupa os últimos pixels do retângulo da lista. Sem reservar esse espaço, o
   * último bloco passa por baixo do header e aparece com a linha final cortada
   * — a rolagem chega ao fim, o corte não é dela.
   *
   * Vem MEDIDO, nunca fixo: é a mesma conta que o header preso já errou uma
   * vez, número fixo contra altura real, e a diferença reaparece exatamente
   * como bloco cortado.
   */
  bottomInset?: number;
  /**
   * Tamanho da fonte do terminal, em px.
   *
   * O corpo do bloco usa a MESMA métrica do terminal: é a mesma saída, e o
   * dono pode ter mudado o tamanho. Ver `getDefaultFontSize`.
   */
  fontSizePx: number;
  /**
   * Altura real de uma linha do terminal, em px.
   *
   * Medida, nunca calculada: o xterm aplica a entrelinha sobre a altura do
   * glifo e o CSS sobre o `font-size`, então o mesmo 1.35 dá 21,5px de um lado
   * e 16,2px do outro. O corpo do bloco é a mesma saída — tem de ter a mesma
   * altura, ou o texto "pula" ao virar cartão.
   */
  lineHeightPx: number;
  /**
   * Largura real de uma célula do terminal, em px.
   *
   * A mesma medida no outro eixo, e ela entra na ESTIMATIVA: o corpo do cartão
   * quebra linha, então uma linha lógica pode ocupar três. Ver `blockMetrics`.
   */
  cellWidthPx: number;
  /**
   * Onde e quando a sessão abriu — o cartão-zero, no começo da pilha.
   *
   * Ausente não é caso de erro: sessão sem cwd conhecido ainda merece a marca de
   * início.
   */
  opened?: { cwd: string | null; atMs: number | null };
}

/**
 * Memoizada por último, e é a memoização que mais rende.
 *
 * O `App` é um componente só, com dezenas de estados, e re-renderiza a ~60 Hz
 * enquanto um comando escreve — a altura da faixa ao vivo é medida a cada
 * repintura do xterm. Sem memo aqui, toda lista de blocos de todo painel
 * re-renderizava junto, a cada frame.
 *
 * > [!warning] Memo só vale com as props estáveis do outro lado. `rect`,
 * > `opened`, `onActivate`, `onPick` e `onClearPick` são objetos e funções: se
 * > o `App` voltar a criá-los inline no JSX, a comparação rasa falha em todo
 * > render e este memo passa a custar sem devolver nada.
 */
export const BlockList = memo(function BlockList({
  blocks,
  rect,
  bottomInset = 0,
  fontSizePx,
  lineHeightPx,
  cellWidthPx,
  opened,
  onInject,
  onActivate,
  marked,
  onPick,
  onClearPick,
  copyCombo,
}: Props) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  // Cor indexada é resolvida na renderização: trocar de tema tem de repintar a
  // saída antiga junto, senão o bloco congela a paleta de quando foi capturado.
  const [, repaint] = useReducer((n: number) => n + 1, 0);
  useEffect(() => onTerminalThemeChange(() => repaint()), []);

  // A largura do scroller, medida.
  //
  // Entra em duas contas: quantos caracteres cabem numa linha do corpo — sem
  // isso a estimativa ignora a quebra — e o descarte das alturas já medidas,
  // logo abaixo. Medida, e não derivada do `rect` em %: o retângulo é uma
  // fração do painel, e a mesma fração dá larguras diferentes conforme a
  // janela e o painel lateral.
  /**
   * Quantas linhas cada bloco está mostrando, por id.
   *
   * Mora na LISTA, não no cartão. No cartão, o estado morria junto com ele na
   * virtualização — expandir, rolar e voltar devolvia o bloco fechado. E, o que
   * é pior, a `estimate` daqui não conseguia enxergá-lo: um bloco expandido de
   * 5 mil linhas continuava estimado em 200, o total da lista ficava muito
   * menor que o conteúdo real, e a rolagem nunca chegava ao fim.
   *
   * Sem limpeza, pelo mesmo motivo de `withEntry`: id de bloco não se repete, e
   * um número por bloco lido é barato perto de varrer isto a cada mudança.
   */
  const [shown, setShown] = useState<Record<number, number>>({});
  const showMore = useCallback((id: number) => {
    setShown((prev) => ({
      ...prev,
      [id]: (prev[id] ?? BODY_LIMIT) + BODY_LIMIT,
    }));
  }, []);

  const [widthPx, setWidthPx] = useState(0);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    setWidthPx(el.clientWidth);
    const ro = new ResizeObserver(() => setWidthPx(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const cols = columnsFor(widthPx, cellWidthPx, BODY_INSET_X_PX);

  // A estimativa sai do número de linhas, não de um número fixo: com 80px
  // chutados, o primeiro layout põe o bloco no lugar errado e ele PULA quando a
  // medição real chega — que é o "aparece com delay e renderiza".
  const estimate = (index: number) => {
    const line = lineHeightPx;
    const block = blocks[index];
    if (!block) return line + HEADER_PX;
    // Cartão de tela cheia não tem corpo, mas tem a linha do MOTIVO. Sem ela na
    // conta, todo bloco de `vim`/`htop` era estimado curto demais.
    const reason = block.altScreen ? ALT_SCREEN_PX : 0;
    // Linhas VISUAIS, não lógicas: o corpo desenha com `whitespace-pre-wrap`, e
    // num painel estreito uma linha de saída ocupa três. Ver `blockMetrics`.
    //
    // O teto é o que o bloco está MOSTRANDO, não `BODY_LIMIT`: quem já pediu
    // mais linhas tem um cartão maior, e estimar pelo teto fixo devolvia um
    // total de lista menor que o conteúdo — a rolagem parava antes do fim.
    const limit = shown[block.id] ?? BODY_LIMIT;
    const body = bodyLines(block.lines, cols, limit) * line;
    const more = block.lines.length > limit ? SHOW_MORE_PX : 0;
    const footer = block.truncated > 0 ? line : 0;
    return HEADER_PX + reason + body + more + footer + BLOCK_GAP_PX;
  };

  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: estimate,
    // Já esteve em 2, para conter os nós de DOM: um cartão de 200 linhas
    // coloridas passa de 3 mil, e a folga sozinha mantinha dezenas de milhares
    // montados fora da tela. Voltou para 6 — folga é o que cobre a área que
    // entra na tela ANTES de o React renderizar, e cortá-la troca nós de DOM
    // por retângulo vazio durante o gesto. Quem paga a conta dos nós agora é o
    // `memo` de `Line` e do cartão, que é onde ela devia ser paga.
    overscan: 6,
    // O cache de altura é keyado por ESTE valor. Sem ele o padrão é o índice —
    // e `mergeBlockHistory` faz prepend do histórico do SQLite, o que desloca
    // todos os índices de uma vez e faz cada altura medida passar a pertencer a
    // outro bloco. É o embaralhar de cartões ao abrir a sessão.
    getItemKey: (index) => blocks[index]?.id ?? index,
  });

  // Métrica nova = TODO cartão muda de altura, e o virtualizador precisa saber.
  //
  // Ele guarda a altura MEDIDA por índice, e `estimateSize` só vale para quem
  // ele ainda não mediu. Sem este reset, um cartão já medido conserva a altura
  // antiga enquanto o corpo passa a desenhar na nova — o bloco seguinte é
  // posicionado por cima do anterior e o texto de um vaza sobre o outro.
  //
  // Não era possível antes: o corpo do cartão tinha tamanho fixo, então a
  // métrica nunca mudava depois do mount e o cache nunca envelhecia. Ela virou
  // dinâmica para o cartão acompanhar a fonte do terminal — e a mudança que
  // consertou o texto pulando ao virar bloco abriu esta.
  //
  // O caminho normal passa por aqui sempre, não só quando o dono mexe na fonte:
  // a altura de linha nasce estimada da fonte e vira o valor medido do
  // `.xterm-screen` assim que o terminal desenha.
  //
  // LARGURA conta pelo mesmo motivo: o corpo quebra linha, então um painel mais
  // estreito deixa TODO cartão mais alto.
  //
  // > [!warning] Os três passos abaixo são um só. Tirar qualquer um recria a
  // > sobreposição de cartões, e o teste não pega — só o pixel na tela.
  //
  // **1. `measure()` sozinho piora o que deveria consertar.** Ele faz
  // `itemSizeCache.clear()` e mais nada (virtual-core, `this.measure`). Quem
  // repõe a medida é o `ResizeObserver` do virtualizador, e observer só fala
  // quando o tamanho MUDA: o cartão já montado que continua do mesmo tamanho
  // nunca reporta de novo e fica com a ESTIMATIVA para sempre. Medido no app:
  // um bloco de 1101px cujo vizinho começava 72px depois.
  //
  // **2. `getTotalSize()` não é chamada morta.** Ela força o recálculo das
  // posições AGORA, com o cache já vazio, o que deixa cada item com a sua
  // estimativa em `measurementsCache`. Sem ela, o passo 3 mede certo e joga
  // fora: `resizeItem` compara o valor medido com o que está em
  // `measurementsCache` — que ainda seria a medida ANTIGA, idêntica à nova —,
  // vê `delta === 0` e não regrava. Como `measure()` já tinha limpado o cache,
  // o item cai de volta na estimativa. Foi assim que a sobreposição sobreviveu
  // à primeira correção: de 1029px de erro para 34px, sem nunca chegar a zero.
  //
  // **3. A re-medição de quem está montado.** `measureElement` com o mesmo nó
  // não re-observa nada, só mede e chama `resizeItem` — que agora grava, porque
  // o delta contra a estimativa não é zero.
  useEffect(() => {
    virtualizer.measure();
    const el = scrollRef.current;
    if (!el) return;
    virtualizer.getTotalSize();
    el.querySelectorAll<HTMLElement>("[data-index]").forEach((node) => {
      virtualizer.measureElement(node);
    });
  }, [fontSizePx, lineHeightPx, cellWidthPx, widthPx, virtualizer]);

  /**
   * Confere o que está na tela contra o que o virtualizador acha, e corrige a
   * diferença. Não apaga nada — ao contrário do bloco acima.
   *
   * Existe porque o virtualizador NÃO mede item que monta durante a rolagem
   * (`shouldMeasureDuringScroll`, em virtual-core): é uma decisão dele, para os
   * itens não pularem sob o ponteiro de quem está rolando. O efeito colateral é
   * que um cartão que entrou na tela rolando fica com a ESTIMATIVA, e estimativa
   * de bloco com texto quebrado erra por uma linha ou mais — que aparece como o
   * cartão seguinte começando por cima do fim do anterior.
   *
   * `resizeItem` compara com o valor em cache e só escreve quando difere, então
   * passar por aqui à toa não custa nem provoca laço de render.
   */
  const settle = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.querySelectorAll<HTMLElement>("[data-index]").forEach((node) => {
      const index = Number(node.dataset.index);
      const size = node.offsetHeight;
      if (Number.isFinite(index) && index >= 0 && size > 0) {
        virtualizer.resizeItem(index, size);
      }
    });
  }, [virtualizer]);

  const last = blocks.length - 1;

  // Rolagem movida na mão, e não pela nativa.
  //
  // A nativa não chegava aqui com o ponteiro sobre os cartões, só sobre a
  // barra: esta lista é uma camada sobreposta ao terminal, e o WebKit prende o
  // gesto ao scroller que escolheu no primeiro evento — preso no de trás, o de
  // cima não recebe mais nada.
  //
  // > [!warning] Listener NATIVO com `passive: false`, e não o `onWheel` do
  // > React. Este efeito inteiro existe por causa dessa diferença.
  // >
  // > O React registra `wheel` no container raiz sempre como PASSIVO (ele trata
  // > `wheel`, `touchstart` e `touchmove` como caso especial). Num listener
  // > passivo o `preventDefault` é inerte: o navegador avisa no console e rola
  // > assim mesmo. Como o handler já tinha somado `deltaY` no `scrollTop` na
  // > mão, a rolagem nativa vinha POR CIMA e o gesto andava o DOBRO — que é a
  // > sensação de "pula e escapa" no trackpad.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (event: WheelEvent) => {
      const before = el.scrollTop;
      el.scrollTop = before + event.deltaY;
      // No topo e no fim o gesto passa adiante, senão a lista viraria um
      // buraco onde nada mais rola.
      if (el.scrollTop !== before) event.preventDefault();
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // Bloco novo entra, a lista rola até ele e itens montam no caminho — o mesmo
  // caso do parágrafo acima, e o mais comum de todos: é o que acontece a cada
  // comando executado. Num frame, para conferir o que o layout já desenhou.
  useEffect(() => {
    if (last < 0) return;
    const id = requestAnimationFrame(settle);
    return () => cancelAnimationFrame(id);
  }, [last, settle]);

  // Estava colado no fim quando o dono mexeu pela última vez?
  //
  // Medir isto na hora em que a altura muda não serve: o efeito roda depois do
  // render, quando a lista JÁ encolheu e o fim já escapou. O que vale é o
  // estado da última rolagem de verdade.
  const atBottomRef = useRef(true);

  // A altura da lista muda a cada linha de saída que chega — ela cede à faixa
  // ao vivo só o que a saída usa. Ancorada embaixo e sem reancorar a rolagem, o
  // último bloco aparece cortado no meio a cada linha nova.
  //
  // Só reancora quem já estava no fim: puxar de volta para baixo quem subiu
  // para ler um bloco antigo é pior do que o corte que isto conserta.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || last < 0 || !atBottomRef.current) return;
    // `scrollTop` na mão, e não `scrollToIndex`: o virtualizador posiciona os
    // itens a partir do topo, mas a lista é ancorada embaixo (`justify-end` com
    // `min-h-full`), então as posições dele não batem com a rolagem — é o mesmo
    // descolamento que o header preso já contorna logo abaixo.
    //
    // > [!warning] Um `scrollToIndex(last)` já morou três linhas acima daqui, e
    // > era ele que travava a rolagem. Não o traga de volta.
    // >
    // > `scrollToIndex` não escreve `scrollTop` e pronto: ele arma um laço de
    // > `requestAnimationFrame` (`scheduleScrollReconcile`, no virtual-core)
    // > que recalcula o alvo A CADA FRAME e reescreve a rolagem até convergir
    // > ou até 5 segundos. Com o descolamento do parágrafo acima, o alvo nunca
    // > bate — então o laço ia até o teto, a cada bloco novo, desfazendo no
    // > frame seguinte todo gesto de quem tentasse subir para ler.
    // >
    // > Ele também não tinha a guarda de `atBottomRef` que este efeito tem, o
    // > que fazia bloco novo puxar a lista para o fim mesmo com o dono lendo
    // > algo antigo lá em cima.
    //
    // Em rAF porque o efeito roda antes de o WebKit refazer o layout com a
    // altura nova: lido agora, `scrollHeight` ainda é o de antes da mudança.
    const id = requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight;
    });
    return () => cancelAnimationFrame(id);
  }, [rect.height, bottomInset, last]);

  // Bloco mais alto que o painel some com o próprio header ao rolar, e o que
  // sobra na tela é uma parede de texto igual à de um terminal comum — some
  // justamente o "cada comando tem o seu recorte". O header do bloco que está
  // no topo da viewport fica preso ali.
  //
  // Preso por fora da lista, e não com `position: sticky`: o virtualizador
  // posiciona cada item com `transform`, e sticky dentro de um ancestral
  // transformado gruda no item, não na viewport.
  const [pinned, setPinned] = useState<number | null>(null);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const sync = () => {
      // Lista que cabe na tela está sempre no fim, por definição.
      atBottomRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_SLACK_PX;
      // Lista que cabe na tela não tem o que prender: ancorada embaixo, ela
      // começa deslocada do topo, e as posições que o virtualizador calcula
      // deixam de bater com a rolagem.
      if (el.scrollHeight <= el.clientHeight + 1) {
        setPinned(null);
        return;
      }
      const top = el.scrollTop;
      const items = virtualizer.getVirtualItems();
      // `<` e não `<=`: com o bloco começando exatamente no topo, o header dele
      // está à vista e prender desenha o MESMO header duas vezes, um debaixo do
      // outro. Prende só quando o de verdade já saiu de cena.
      //
      // E só bloco ALTO prende, que é o caso para o qual isto existe: o header
      // sumir ao rolar só é problema quando ainda falta muita saída embaixo.
      //
      // Sem o piso, a barra piscava várias vezes por segundo numa pilha de
      // comandos curtos. A conta: a condição acima só vale numa janela de
      // `altura − HEADER_PX`, então um bloco de `ls` com 43px de altura a
      // satisfaz por 8px de cada 43 — aparece, some, aparece, a cada bloco
      // cruzado. Alto o bastante, a janela é quase o bloco inteiro e a barra
      // fica parada, que é como ela deve se comportar.
      const current = items.find(
        (item) =>
          item.end - item.start >= PIN_MIN_PX &&
          item.start < top &&
          item.end > top + HEADER_PX,
      );
      setPinned(current ? current.index : null);
    };
    sync();
    // A conferência de alturas roda quando a rolagem PARA, não a cada evento:
    // durante o gesto o virtualizador não mede de propósito, e corrigir no meio
    // do caminho é justamente o pulo sob o ponteiro que ele evita.
    let settleTimer: number | null = null;
    const onScroll = () => {
      sync();
      if (settleTimer !== null) window.clearTimeout(settleTimer);
      settleTimer = window.setTimeout(settle, SETTLE_MS);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      el.removeEventListener("scroll", onScroll);
      if (settleTimer !== null) window.clearTimeout(settleTimer);
    };
  }, [virtualizer, blocks.length, settle]);

  return (
    // Duas camadas: a lista que rola e, POR CIMA dela, o que fica parado.
    //
    // O header preso já morou dentro do scroll, seguro por `sticky` e uma
    // margem negativa que o tirava do fluxo. A conta nunca fecha: a margem é um
    // número fixo, a altura do header é medida, e a diferença aparece como a
    // primeira linha do bloco cortada e como scroll que não chega ao fim.
    // Sobreposição não é fluxo — então não fica no fluxo.
    <div
      className="pointer-events-none"
      style={{
        position: "absolute",
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        // Encurtado pelo header do bloco em execução, que é desenhado sobre o
        // fim desta lista. Ver `bottomInset`.
        height: `calc(${rect.height}% - ${bottomInset}px)`,
      }}
    >
      <div
        ref={scrollRef}
        onMouseDown={onActivate}
        // Só o fundo: clique que veio de um cartão para aqui por bubbling já
        // decidiu o que fazer com a seleção.
        onClick={(event) => {
          if (event.target === event.currentTarget) onClearPick?.();
        }}
        // `overscroll-contain`: ao bater no topo, o WebKit passa o gesto para o
        // ancestral e TRAVA nele até a rolagem terminar — daí "rolei tudo para
        // cima e agora não desce mais, só clicando na barra". Conter o encadea-
        // mento mantém o gesto nesta lista.
        // Sem contorno próprio: quem emoldura é o PAINEL, uma camada acima de
        // tudo (ver `App`). Lista e terminal desenhando cada um a sua borda
        // punham duas caixas dentro do mesmo painel, e a de baixo só existia na
        // metade que o terminal ocupa.
        className="pointer-events-auto h-full overflow-y-auto overscroll-contain rounded-[4px] bg-tyba-sunken px-2 pb-3 pt-2"
      >
        {/* Ancorado embaixo, como todo terminal: os blocos crescem de baixo
            para cima e o último fica colado na linha de comando. Ancorado em
            cima, poucos blocos deixam um vazio enorme logo acima do input —
            que é exatamente para onde o olho vai. */}
        <div className="flex min-h-full flex-col justify-end">
          {opened && (
            <SessionOpened
              cwd={opened.cwd}
              at={opened.atMs}
              fontSizePx={fontSizePx}
            />
          )}
          <div
            style={{ height: virtualizer.getTotalSize(), position: "relative" }}
          >
          {virtualizer.getVirtualItems().map((item) => (
            <div
              key={blocks[item.index].id}
              ref={virtualizer.measureElement}
              data-index={item.index}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${item.start}px)`,
                // O respiro é PADDING daqui, não margem do cartão.
                //
                // `measureElement` lê `offsetHeight`, e margem de filho colapsa
                // para fora dele: o virtualizador media o cartão sem o respiro,
                // punha o próximo colado, e a margem que ele não viu virava
                // sobreposição. Como padding, o respiro está dentro do que ele
                // mede — e a estimativa passa a falar da mesma caixa.
                paddingBottom: BLOCK_GAP_PX,
              }}
            >
              <BlockCard
                block={blocks[item.index]}
                fontSizePx={fontSizePx}
                lineHeightPx={lineHeightPx}
                onInject={onInject}
                marked={marked?.has(blocks[item.index].id) ?? false}
                // Repassado direto, sem amarrar o id aqui: ver o comentário do
                // prop em `BlockCard`.
                onPick={onPick}
                shown={shown[blocks[item.index].id] ?? BODY_LIMIT}
                onShowMore={showMore}
                />
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Faixa rente ao topo, e não cartão flutuante.
          Ela morava em `inset-x-2 top-2` com borda em volta, cantos e
          `shadow-md`: lia como um cartão solto voando por cima da lista, e
          entre a sombra e as margens comia a linha de cima E a de baixo do que
          estava embaixo dela. Encostada nas bordas, com um filete só
          separando, ela vira o que é — cromo da lista, dizendo de quem é a
          saída que se está lendo.
          Opaca continua sendo: o trabalho dela é esconder o que passa por trás.
          O `px-2` repete o padding do scroller para o comando cair na mesma
          coluna do comando de todo cartão. */}
      {pinned !== null && blocks[pinned] && (
        <div className="absolute inset-x-0 top-0 z-10 rounded-t-[4px] border-b border-tyba-border bg-tyba-sunken px-2">
          <BlockHeader block={blocks[pinned]} pinned onInject={onInject} />
        </div>
      )}

      {/* Marcar bloco é um modo, e modo que não se anuncia ninguém acha. */}
      {marked && marked.size > 0 && (
        <div className="absolute bottom-4 right-3 z-10">
          <span className="rounded-full border border-tyba-border bg-tyba-surface px-2 py-0.5 font-mono text-[10px] text-tyba-text-muted shadow-md">
            {t("blockPicked", { count: marked.size, combo: copyCombo ?? "" })}
          </span>
        </div>
      )}
    </div>
  );
});

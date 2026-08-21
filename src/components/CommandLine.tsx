import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  submitShellLine,
  suggestLine,
  writeControl,
  type CommandSuggestion,
  type SessionId,
} from "../lib/ipc";
import { shortPath } from "../lib/blockText";
import { toastError } from "../lib/toast";
import {
  boxIsMounted,
  clearsDraft,
  controlBytes,
  ghostFor,
  lineToken,
  type LineState,
  pathToken,
  replaceToken,
  SUGGEST_DEBOUNCE_MS,
} from "../lib/commandLine";

const MAX_HEIGHT_PX = 140;

const PLACEHOLDER_BY_STATE: Record<LineState, string> = {
  own: "commandLinePlaceholder",
  waiting: "commandLineWaiting",
  continuation: "commandLineContinuation",
  running: "commandLineRunning",
  app: "commandLineApp",
  off: "commandLineOff",
};

interface Props {
  sessionId: SessionId;
  /** Onde o comando vai rodar. Ver o rodapé da linha. */
  cwd: string | null;
  /**
   * Quem está com a tela, quando `state === "app"`. Ver `programName`.
   *
   * "nvim está no controle" é reconhecível — o usuário abriu o nvim. "Um app
   * está usando a tela" é verdadeiro e não ajuda ninguém.
   */
  program?: string | null;
  /**
   * Pode encolher a faixa quando um app toma a tela?
   *
   * Só com UM painel. Existe uma linha de comando para a sessão ativa, não uma
   * por painel: encolher com a tela dividida faria alternar o foco entre um
   * painel em vim e outro no prompt mudar a altura da faixa a cada troca — e
   * altura da faixa é altura da área de painéis, logo um `resizeSession` em
   * TODOS os PTYs por troca de foco. É a mesma família do bug que a regra "a
   * linha nunca some" existe para evitar.
   *
   * Cai sozinho quando existir uma linha por painel: aí cada faixa encolhe a
   * sua e só aquele PTY sente.
   */
  canCollapse?: boolean;
  scope: { cwd: string | null; repoRoot: string | null };
  /** Muda quando a linha volta a ser do TYBA (fim de comando, saída do vim). */
  focusNonce: number;
  /**
   * Por que a linha não é editável agora. Ela **nunca** desaparece: sumir e
   * voltar a cada comando redimensionava o terminal duas vezes por execução.
   */
  state: LineState;
  /**
   * Texto vindo de fora (paleta de histórico, snippet, colar).
   *
   * Com o terminal somente-leitura, injetar via `term.paste` seria engolido em
   * silêncio — o destino passa a ser a caixa, que é quem edita a linha.
   */
  inject?: { text: string; nonce: number } | null;
}

/**
 * A linha de comando do shell.
 *
 * Deliberadamente separada do `RichInput`: aquele é a caixa de prompt de
 * agente, com regras opostas — multiline por padrão, `@arquivo`, aviso de
 * prompt sensível e `⌘↵` para enviar. Numa linha de comando o Enter executa e
 * não existe botão de enviar.
 */
export function CommandLine({
  sessionId,
  cwd,
  program,
  canCollapse = true,
  scope,
  focusNonce,
  state,
  inject,
}: Props) {
  const waiting = state !== "own";
  // App de tela cheia: a linha não desaparece, troca de conteúdo. E encolhe,
  // se houver espaço para isso sem mexer nos painéis — ver `canCollapse`.
  const collapsed = !boxIsMounted(state);
  const compact = collapsed && canCollapse;
  const where = shortPath(cwd);
  const { t } = useTranslation();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState("");
  const [caret, setCaret] = useState(0);
  const [hits, setHits] = useState<CommandSuggestion[]>([]);
  const [index, setIndex] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);
  // Só para o anel: o foco do DOM já está na caixa, mas o CSS `:focus-within`
  // não alcança um irmão, e o anel mora na moldura em volta.
  const [focused, setFocused] = useState(false);
  const [paths, setPaths] = useState<string[]>([]);
  const [args, setArgs] = useState<string[]>([]);

  const seenInject = useRef(inject?.nonce ?? 0);
  useEffect(() => {
    if (!inject || inject.nonce === seenInject.current) return;
    seenInject.current = inject.nonce;
    setText(inject.text);
    setCaret(inject.text.length);
    setMenuOpen(false);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(inject.text.length, inject.text.length);
    });
  }, [inject]);

  const seenNonce = useRef(focusNonce);
  useEffect(() => {
    if (focusNonce === seenNonce.current) return;
    seenNonce.current = focusNonce;
    inputRef.current?.focus();
  }, [focusNonce]);

  const token = pathToken(text, caret);
  const arg = lineToken(text, caret);

  // Uma chamada por tecla. Antes eram três `invoke` — histórico, caminho e
  // argumento — atravessando a ponte do webview a cada digitação para uma única
  // mudança na tela.
  useEffect(() => {
    if (waiting || !text.trim()) {
      setHits([]);
      setPaths([]);
      setArgs([]);
      setMenuOpen(false);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      void suggestLine({
        query: text,
        cwd: scope.cwd,
        repoRoot: scope.repoRoot,
        pathToken: token?.value ?? null,
        argPrefix: arg?.prefix ?? null,
        argToken: arg?.value ?? null,
      })
        .then((found) => {
          if (!alive) return;
          setHits(found.commands);
          setPaths(found.paths);
          setArgs(found.arguments);
          setIndex(0);
        })
        .catch(() => {
          if (!alive) return;
          setHits([]);
          setPaths([]);
          setArgs([]);
        });
    }, SUGGEST_DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waiting, text, token?.value, arg?.prefix, arg?.value, scope.cwd, scope.repoRoot]);

  // ↑ abre o histórico. Com a caixa vazia não há o que sugerir enquanto se
  // digita, então a lista só é buscada quando alguém pede — que é o gesto que
  // todo mundo já traz do shell.
  const openHistory = () => {
    void suggestLine({
      query: text,
      cwd: scope.cwd,
      repoRoot: scope.repoRoot,
      pathToken: null,
      argPrefix: null,
      argToken: null,
    })
      .then((found) => {
        if (found.commands.length === 0) return;
        setHits(found.commands);
        setIndex(0);
        setMenuOpen(true);
      })
      .catch(() => {});
  };

  const takeArg = (completion: string) => {
    if (!arg) return false;
    const next = replaceToken(text, arg, completion);
    apply(next.text, next.caret);
    setArgs([]);
    setMenuOpen(false);
    return true;
  };

  const takePath = (completion: string) => {
    if (!token) return false;
    const next = replaceToken(text, token, completion);
    apply(next.text, next.caret);
    setPaths([]);
    setMenuOpen(false);
    return true;
  };

  // Comando que só falhou completa no cinza quando é prefixo do que se está
  // digitando, mas nunca é oferecido como item — devolver `lljh` numa lista é
  // sugerir o próprio erro de digitação.
  const listed = hits.filter((hit) => !hit.failed);
  // Caminho ganha do histórico no cinza: quem já digitou `cd sr` está
  // escolhendo um diretório, não repetindo um comando inteiro.
  const pathGhost =
    token && paths.length > 0 && paths[0].startsWith(token.value)
      ? paths[0].slice(token.value.length)
      : "";
  // Flag nunca é caminho: `--l` não existe em disco, e tentar completá-lo como
  // arquivo só produziria silêncio.
  const argGhost =
    arg && args.length > 0 && args[0].startsWith(arg.value)
      ? args[0].slice(arg.value.length)
      : "";
  const ghost = pathGhost || argGhost || ghostFor(text, hits);
  const showMenu =
    menuOpen && (listed.length > 0 || paths.length > 0 || args.length > 0);

  const resize = () => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT_PX)}px`;
  };

  // A altura da caixa é medida, não declarada: `rows={1}` e `min-h-[28px]` só
  // dão o piso, e quem sabe que o rascunho tem três linhas é o `scrollHeight`.
  // Por isso ela se recalcula aqui, em UM lugar, e não em cada caminho que
  // mexe no texto. Um lugar só derruba as duas armadilhas de uma vez:
  //
  // - a caixa não some quando a linha deixa de ser sua. Em `running`,
  //   `continuation` e `off` é a MESMA textarea, desabilitada, com o rascunho
  //   dentro (ver `boxIsMounted`). Zerar a altura na ida colapsava o texto para
  //   uma linha, e nada o devolvia na volta: altura inline não se desfaz
  //   sozinha, e quem a escrevia só rodava ao digitar.
  // - saindo do `app` a textarea é REMONTADA — elemento novo, sem altura
  //   inline, com o rascunho ainda no estado. Sem remedir por `state`, voltar
  //   do `vim` devolvia a caixa em 28px com o texto cortado.
  //
  // `useLayoutEffect` porque a medida acontece antes da pintura: com `useEffect`
  // o quadro intermediário com a altura errada chega à tela.
  useLayoutEffect(resize, [state, text]);

  const apply = (next: string, nextCaret: number) => {
    setText(next);
    setCaret(nextCaret);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.setSelectionRange(nextCaret, nextCaret);
    });
  };

  const acceptGhost = () => {
    if (!ghost) return false;
    if (pathGhost && token) return takePath(token.value + pathGhost);
    if (argGhost && arg) return takeArg(arg.value + argGhost);
    apply(text + ghost, text.length + ghost.length);
    return true;
  };

  const run = () => {
    const value = text;
    if (!value.trim()) return;
    setMenuOpen(false);
    // A linha só é limpa quando o shell aceitou. Multiline sem bracketed paste
    // é recusado pelo core, e engolir o erro apagaria o que o usuário escreveu
    // sem executar nada.
    void submitShellLine(sessionId, value)
      .then(() => {
        apply("", 0);
        setHits([]);
      })
      .catch((error) => toastError(t("commandLineFailed"), error));
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (waiting) return;
    const bytes = controlBytes({
      key: e.key,
      ctrl: e.ctrlKey,
      meta: e.metaKey,
      alt: e.altKey,
    });
    if (bytes) {
      e.preventDefault();
      void writeControl(sessionId, bytes).catch(() => {});
      if (clearsDraft({ key: e.key, ctrl: true, meta: false, alt: false })) {
        apply("", 0);
        setMenuOpen(false);
      }
      return;
    }

    if (showMenu && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      const delta = e.key === "ArrowDown" ? 1 : -1;
      if (listed.length === 0) return;
      setIndex((prev) => (prev + delta + listed.length) % listed.length);
      return;
    }
    if (
      e.key === "ArrowDown" &&
      !showMenu &&
      (listed.length > 0 || paths.length > 0 || args.length > 0)
    ) {
      e.preventDefault();
      setMenuOpen(true);
      return;
    }
    // ↑ com a lista fechada é o histórico, como em qualquer shell. Já filtrado
    // pelo que estiver escrito: com a caixa vazia vem o mais recente.
    if (e.key === "ArrowUp" && !showMenu) {
      e.preventDefault();
      if (listed.length > 0) {
        setIndex(0);
        setMenuOpen(true);
      } else {
        openHistory();
      }
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      setMenuOpen(false);
      return;
    }
    if (e.key === "Tab") {
      e.preventDefault();
      if (showMenu) {
        apply(listed[index].command, listed[index].command.length);
        setMenuOpen(false);
        return;
      }
      if (
        !acceptGhost() &&
        (listed.length > 0 || paths.length > 0 || args.length > 0)
      ) {
        setMenuOpen(true);
      }
      return;
    }
    // → no fim da linha aceita o cinza, como no zsh-autosuggestions; no meio do
    // texto continua sendo só mover o cursor.
    if (e.key === "ArrowRight" && caret === text.length && acceptGhost()) {
      e.preventDefault();
      return;
    }
    if (e.key === "Enter") {
      if (e.shiftKey) return;
      e.preventDefault();
      if (showMenu && listed.length > 0) {
        apply(listed[index].command, listed[index].command.length);
        setMenuOpen(false);
        return;
      }
      run();
    }
  };

  return (
    // A caixa é OUTRA peça, não o fim do output.
    //
    // Ela morava colada no rodapé, com o mesmo `bg-tyba-sunken` do painel e uma
    // borda de 7% de opacidade separando as duas. Mesma cor, mesma fonte, mesmo
    // tamanho: nada dizia onde se digita. Sobe uma camada (`raised`), ganha
    // respiro em volta e um anel que acende no foco — o mesmo verde da moldura
    // do painel ativo, que é a peça de linguagem que já significa "é aqui".
    // A FAIXA continua na cor da área de terminal — sem isso ela mostraria o
    // fundo do app (mais claro, e com a aurora por cima), trocando um degrau por
    // outro. Quem se destaca é a caixa dentro dela, não a faixa.
    //
    // `px-2` e não `px-3`: é o mesmo recuo do scroller da lista, então a borda
    // da caixa cai na MESMA coluna da borda dos cartões. Com 12px contra 8px a
    // diferença era de 4px — pouca para parecer intencional, suficiente para
    // parecer torto.
    //
    // Respiro igual em cima e embaixo: com `pt-1 pb-2` a caixa ficava encostada
    // na lista e solta do rodapé.
    // A separação da lista é um FILETE, não um degrau.
    //
    // A faixa continua na cor da área de terminal — sem isso ela mostraria o
    // fundo do app, mais claro e com a aurora por cima, trocando uma emenda por
    // outra. O que marca onde a leitura acaba e a escrita começa é uma linha de
    // 1px, que acende de leve quando a linha é sua: é o segundo sinal de foco,
    // junto com o chevron.
    //
    // `px-0` na faixa e `px-2.5` na linha: o chevron cai na mesma coluna do
    // chevron de todo bloco, que é o alinhamento que faz as duas coisas lerem
    // como a mesma família.
    <div
      className={`relative shrink-0 border-t bg-tyba-sunken px-2 py-1.5 transition-colors ${
        focused ? "border-tyba-green/25" : "border-tyba-border"
      }`}
    >
      {showMenu && (
        <div className="absolute bottom-full left-2.5 right-2.5 z-20 mb-1 max-h-56 overflow-y-auto rounded-[6px] border border-tyba-border bg-tyba-raised py-1 shadow-lg">
          {args.map((candidate) => (
            <button
              key={`arg:${candidate}`}
              onMouseDown={(event) => {
                event.preventDefault();
                takeArg(candidate);
              }}
              className="flex w-full items-center gap-2 px-2.5 py-1 text-left font-mono text-[12px] text-tyba-text-muted hover:bg-tyba-text/[.04]"
            >
              <span className="min-w-0 flex-1 truncate">{candidate}</span>
            </button>
          ))}
          {paths.map((candidate) => (
            <button
              key={`path:${candidate}`}
              onMouseDown={(event) => {
                event.preventDefault();
                takePath(candidate);
              }}
              className="flex w-full items-center gap-2 px-2.5 py-1 text-left font-mono text-[12px] text-tyba-text-muted hover:bg-tyba-text/[.04]"
            >
              <span className="min-w-0 flex-1 truncate">{candidate}</span>
            </button>
          ))}
          {listed.map((hit, i) => (
            <button
              key={`${hit.kind}:${hit.command}`}
              onMouseDown={(event) => {
                event.preventDefault();
                apply(hit.command, hit.command.length);
                setMenuOpen(false);
              }}
              className={`flex w-full items-center gap-2 px-2.5 py-1 text-left font-mono text-[12px] ${
                i === index
                  ? "bg-tyba-green/15 text-tyba-text"
                  : "text-tyba-text-muted hover:bg-tyba-text/[.04]"
              }`}
            >
              <span className="min-w-0 flex-1 truncate">{hit.command}</span>
              {hit.kind === "snippet" && (
                <span className="shrink-0 rounded-[3px] border border-tyba-border px-1 text-[9px] text-tyba-text-faint">
                  {hit.label ?? t("paletteSnippets")}
                </span>
              )}
            </button>
          ))}
        </div>
      )}

      <div
        // Não é uma caixa. É a próxima linha do terminal.
        //
        // Antes era um campo elevado: fundo `raised` com verniz vertical
        // (`--tyba-sheen`), aresta iluminada, cantos de 8px, sombra e um anel
        // verde no foco. Cada uma dessas peças existe no design system para
        // separar uma superfície da outra — e é justamente o que aqui não se
        // quer. Sobre o preto absoluto da área de terminal, o verniz lê como
        // plástico, e o conjunto lê como formulário colado no rodapé.
        //
        // A lista logo acima já fixou a gramática de "um comando": chevron
        // verde à esquerda, o texto em mono, o cwd apagado à direita. Esta
        // linha usa a MESMA, e a diferença entre ela e um bloco pronto passa a
        // ser só o cursor piscando — que é a verdade: é o próximo bloco, ainda
        // sendo escrito.
        className="flex items-start gap-2.5 px-2.5"
      >
        {/* O chevron é a âncora e o único elemento colorido da linha.
            Ele também é o indicador de foco: some o anel em volta da caixa, e
            quem diz "o teclado é seu" é ele acendendo. Verde apagado quando a
            linha não é sua, cheio e com brilho quando é — o mesmo verde que o
            header de cada bloco usa para o comando que deu certo. */}
        <span
          aria-hidden
          className={`shrink-0 select-none font-mono transition-all ${
            collapsed
              ? `text-[11px] text-tyba-text-faint ${compact ? "leading-[17px]" : "flex min-h-[28px] items-center"}`
              : `pt-1 text-[13px] leading-[20px] ${focused ? "text-tyba-green" : "text-tyba-green/45"}`
          }`}
          style={
            focused && !collapsed
              ? { textShadow: "var(--tyba-glow-green)" }
              : undefined
          }
        >
          ❯
        </span>

        {collapsed ? (
          /* Colapsada, não escondida.
             A regra de nunca sumir continua valendo — sumir e voltar
             redimensionava o terminal duas vezes por comando e o vim reabria
             com outra altura. O que muda é o que ela desenha por dentro: some
             a caixa de digitar, some o caminho, e sobra uma faixa de uma linha
             dizendo de quem é o teclado.
             O custo de resize é zero aqui: entrar em alt-screen JÁ redimensiona
             o painel inteiro (o terminal vai de metade para tudo), e as duas
             coisas acontecem no mesmo commit do React — um resize, não dois. */
          <span
            className={`flex min-w-0 flex-1 items-center truncate font-mono text-[11px] text-tyba-text-faint ${
              compact ? "leading-[17px]" : "min-h-[28px]"
            }`}
          >
            {program
              ? t("commandLineAppNamed", { program })
              : t("commandLineApp")}
          </span>
        ) : (
        <div className="relative min-w-0 flex-1">
          {ghost && (
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words py-1 font-mono text-[13px] text-transparent"
            >
              {text}
              <span className="text-tyba-text-faint">{ghost}</span>
            </div>
          )}
          <textarea
            ref={inputRef}
            autoFocus
            rows={1}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={text}
            disabled={waiting}
            placeholder={t(PLACEHOLDER_BY_STATE[state])}
            onChange={(e) => {
              setText(e.target.value);
              setCaret(e.target.selectionStart ?? 0);
              setMenuOpen(false);
            }}
            onKeyDown={onKeyDown}
            onKeyUp={() => setCaret(inputRef.current?.selectionStart ?? 0)}
            onClick={() => setCaret(inputRef.current?.selectionStart ?? 0)}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            className="max-h-[140px] min-h-[28px] w-full resize-none border-0 bg-transparent py-1 font-mono text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </div>
        )}

        {/* Onde o comando vai rodar.
            Encurtado com a MESMA regra do header de cada bloco (`…/pai/pasta`),
            e não só o nome da pasta: duas pastas `src` em repositórios
            diferentes são indistinguíveis pelo basename, e é exatamente na hora
            de apertar Enter que essa distinção importa.
            A branch fica de fora — ela está na status bar 20px abaixo e não
            muda o destino do comando. O caminho, sim. */}
        {where && !collapsed && (
          <span
            title={cwd ?? undefined}
            className="shrink-0 truncate pt-1.5 font-mono text-[11px] leading-[17px] text-tyba-text-faint"
          >
            {where}
          </span>
        )}
      </div>
    </div>
  );
}

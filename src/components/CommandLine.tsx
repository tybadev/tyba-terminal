import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  submitShellLine,
  suggestLine,
  type BinarySuggestion,
  type ArgumentSuggestion,
  writeControl,
  type CommandSuggestion,
  type SessionId,
} from "../lib/ipc";
import { shortPath } from "../lib/blockText";
import { toastError } from "../lib/toast";
import {
  boxAcceptsTyping,
  boxIsMounted,
  clearsDraft,
  controlBytes,
  ghostDoToken,
  ghostFor,
  commandPrefix,
  enterAplicaSugestao,
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
  /**
   * O que estava digitado e não foi enviado, guardado por SESSÃO.
   *
   * A caixa é uma só para o workspace inteiro e remonta a cada troca de
   * sessão (a `key` no `App` inclui o id, de propósito: sugestões, histórico
   * e caret são de uma sessão só e não podem vazar para a outra). O efeito
   * colateral era o rascunho morrer junto — meio comando escrito num pane,
   * um clique no pane ao lado, e ele sumia sem aviso. Vale para troca de
   * pane, de aba e de sessão.
   *
   * Vem de fora e não de um estado interno porque quem sobrevive à
   * remontagem tem de morar acima dela.
   */
  draft?: string;
  /**
   * Onde devolver o rascunho. É um `ref` do lado do `App`, não estado: a
   * caixa avisa a cada tecla, e re-renderizar o `App` inteiro por tecla
   * digitada custaria caro sem ninguém precisar do valor até a remontagem.
   */
  onDraftChange?: (sessionId: SessionId, text: string) => void;
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
  draft = "",
  onDraftChange,
}: Props) {
  // Não é "a linha não é minha", é "a caixa não aceita tecla". A diferença é
  // o `waiting`: a linha ainda não é do TYBA, mas o rascunho pode ser escrito
  // e o Enter fica de pé — ver `boxAcceptsTyping`.
  const waiting = !boxAcceptsTyping(state);
  // App de tela cheia: a linha não desaparece, troca de conteúdo. E encolhe,
  // se houver espaço para isso sem mexer nos painéis — ver `canCollapse`.
  const collapsed = !boxIsMounted(state);
  const compact = collapsed && canCollapse;
  const where = shortPath(cwd);
  const { t } = useTranslation();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState(draft);
  // No fim do rascunho, não no começo. Voltar para um pane e digitar no meio
  // do que já estava escrito seria pior que ter perdido o texto: o comando sai
  // errado e a culpa não é óbvia. Zero só quando não há rascunho.
  const [caret, setCaret] = useState(draft.length);
  const [hits, setHits] = useState<CommandSuggestion[]>([]);
  // -1 = NADA escolhido. A lista pode estar visível sem seleção — é o estado
  // em que ela abriu sozinha, e nele o Enter roda o que está escrito.
  const [index, setIndex] = useState(-1);
  const [menuOpen, setMenuOpen] = useState(false);
  // Esc não pode ser desfeito pela própria abertura automática: sem isto a
  // lista reabriria no próximo caractere e o Esc viraria um piscar.
  const [dispensada, setDispensada] = useState(false);
  // Só para o anel: o foco do DOM já está na caixa, mas o CSS `:focus-within`
  // não alcança um irmão, e o anel mora na moldura em volta.
  const [focused, setFocused] = useState(false);
  const [paths, setPaths] = useState<string[]>([]);
  const [args, setArgs] = useState<ArgumentSuggestion[]>([]);
  // Comandos que existem na sessão — `$PATH` e o que o shell contou. Só chega
  // preenchido quando o caret está no primeiro token.
  const [bins, setBins] = useState<BinarySuggestion[]>([]);
  // Uma submissão de cada vez. Ela deixou de ser instantânea: em sessão nova o
  // core segura a linha até o shell abrir a dele, e a caixa só é limpa quando
  // ele aceita. Sem esta trava, um segundo Enter na espera enviaria o MESMO
  // texto de novo — e o shell rodaria o comando duas vezes.
  const [submitting, setSubmitting] = useState(false);

  // O `caret` do React é só para as sugestões; quem posiciona o cursor de
  // verdade é o DOM, e a textarea nasce com a seleção em zero. Sem isto o
  // rascunho voltava com o cursor antes da primeira letra.
  useEffect(() => {
    if (!draft) return;
    const el = inputRef.current;
    if (!el) return;
    el.setSelectionRange(draft.length, draft.length);
    // Só na montagem: depois disso quem manda no cursor é quem está digitando.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
  // Não-nulo só no PRIMEIRO token: passado ele, quem responde é caminho ou
  // argumento, e oferecer um comando ali seria sugerir `pnpm` como valor de flag.
  const cmdPrefix = commandPrefix(text, caret);

  // Uma chamada por tecla. Antes eram três `invoke` — histórico, caminho e
  // argumento — atravessando a ponte do webview a cada digitação para uma única
  // mudança na tela.
  useEffect(() => {
    if (waiting || !text.trim()) {
      setHits([]);
      setPaths([]);
      setArgs([]);
      setBins([]);
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
        sessionId,
        commandPrefix: cmdPrefix,
      })
        .then((found) => {
          if (!alive) return;
          setHits(found.commands);
          setPaths(found.paths);
          setArgs(found.arguments);
          setBins(found.binaries);
          setIndex(-1);
        })
        .catch(() => {
          if (!alive) return;
          setHits([]);
          setPaths([]);
          setArgs([]);
          setBins([]);
        });
    }, SUGGEST_DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [waiting, text, token?.value, arg?.prefix, arg?.value, cmdPrefix, sessionId, scope.cwd, scope.repoRoot]);

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
      sessionId,
      // ↑ abre o HISTÓRICO. A lista de comandos não entra aqui: o gesto pede o
      // que já foi rodado, não o que existe na máquina.
      commandPrefix: null,
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

  const takeBinary = (name: string) => {
    if (!cmdPrefix) return false;
    // Substitui só o primeiro token: editar o começo de uma linha já escrita
    // (`pn install algo` → `pnpm install algo`) não pode apagar o resto.
    const start = text.length - text.trimStart().length;
    const next = `${text.slice(0, start)}${name}${text.slice(start + cmdPrefix.length)}`;
    apply(next, start + name.length);
    setBins([]);
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
    arg && args.length > 0 && args[0].value.startsWith(arg.value)
      ? args[0].value.slice(arg.value.length)
      : "";
  // Comando ganha do histórico e perde para caminho e argumento. Quem digitou
  // `pn` na coluna 1 está escolhendo um COMANDO — e o histórico, que completa a
  // linha inteira, ali só acertaria por coincidência.
  const binGhost =
    cmdPrefix && bins.length > 0 && bins[0].name.startsWith(cmdPrefix)
      ? bins[0].name.slice(cmdPrefix.length)
      : "";
  // Caminho perdia para nada e ganhava de tudo — e num home com `skills/`,
  // digitar `openssl s` virava `openssl skills/` em vez de `s_client`. O nome de
  // pasta é palpite genérico sobre o diretório; a base sabe o que o COMANDO
  // oferece, e conhecimento específico ganha. Só que quem digitou `./s` já disse
  // que quer arquivo, e ali a ordem se inverte de volta.
  const escolhido = ghostDoToken(token?.value ?? "", pathGhost, argGhost);
  const ghost = escolhido.texto || binGhost || ghostFor(text, hits);
  const temItem =
    listed.length > 0 || paths.length > 0 || args.length > 0 || bins.length > 0;
  // A lista se anuncia enquanto se digita o PRIMEIRO token, e só ali. Antes,
  // ela só existia atrás de `Tab` — e ninguém aperta Tab para descobrir algo
  // que não sabe que existe. Passado o primeiro token o cinza já basta: ali o
  // usuário sabe o que está completando.
  const anuncia = !dispensada && cmdPrefix !== null && bins.length > 0;
  const showMenu = (menuOpen || anuncia) && temItem;

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

  // Um efeito, e não um `setText` embrulhado: assim nenhum dos oito pontos que
  // escrevem no texto precisa lembrar de avisar, e o valor guardado é sempre o
  // que foi COMMITADO — inclusive o `""` de depois de enviar, que é o que
  // limpa o rascunho sem ninguém pedir.
  useEffect(() => {
    onDraftChange?.(sessionId, text);
  }, [sessionId, text, onDraftChange]);

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
    // A FONTE vem de `ghostDoToken` em vez de ser redecidida aqui: enquanto os
    // dois lugares refaziam a conta, havia como o cinza mostrar uma coisa e o
    // `Tab` aplicar outra.
    if (escolhido.fonte === "caminho" && token) {
      return takePath(token.value + escolhido.texto);
    }
    if (escolhido.fonte === "argumento" && arg) {
      return takeArg(arg.value + escolhido.texto);
    }
    apply(text + ghost, text.length + ghost.length);
    return true;
  };

  const run = () => {
    const value = text;
    if (!value.trim() || submitting) return;
    setMenuOpen(false);
    setSubmitting(true);
    // A linha só é limpa quando o shell aceitou. Multiline sem bracketed paste
    // é recusado pelo core, e engolir o erro apagaria o que o usuário escreveu
    // sem executar nada.
    void submitShellLine(sessionId, value)
      .then(() => {
        apply("", 0);
        setHits([]);
      })
      .catch((error) => toastError(t("commandLineFailed"), error))
      .finally(() => setSubmitting(false));
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
      if (listed.length === 0) return;
      // Entrar na lista é o gesto que autoriza o Enter a aplicar. Vindo de
      // `-1`, a primeira seta escolhe o primeiro item em vez de pular para o
      // último.
      setMenuOpen(true);
      setIndex((prev) => {
        if (prev < 0) return e.key === "ArrowDown" ? 0 : listed.length - 1;
        const delta = e.key === "ArrowDown" ? 1 : -1;
        return (prev + delta + listed.length) % listed.length;
      });
      return;
    }
    if (
      e.key === "ArrowDown" &&
      !showMenu &&
      (listed.length > 0 ||
        paths.length > 0 ||
        args.length > 0 ||
        bins.length > 0)
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
      setDispensada(true);
      setIndex(-1);
      return;
    }
    if (e.key === "Tab") {
      e.preventDefault();
      if (showMenu && listed.length > 0) {
        // Tab é gesto explícito: aqui aplicar é o que se pede. Sem seleção,
        // aplica o primeiro — indexar `-1` daria `undefined`.
        const escolhido = listed[index >= 0 ? index : 0];
        apply(escolhido.command, escolhido.command.length);
        setMenuOpen(false);
        return;
      }
      if (
        !acceptGhost() &&
        (listed.length > 0 ||
          paths.length > 0 ||
          args.length > 0 ||
          bins.length > 0)
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
      if (
        enterAplicaSugestao({ listaVisivel: showMenu, selecionado: index }) &&
        listed.length > 0
      ) {
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
              key={`arg:${candidate.value}`}
              onMouseDown={(event) => {
                event.preventDefault();
                takeArg(candidate.value);
              }}
              className="flex w-full items-center gap-2 px-2.5 py-1 text-left font-mono text-[12px] text-tyba-text-muted hover:bg-tyba-text/[.04]"
            >
              <span className="shrink-0">{candidate.value}</span>
              {/* A descrição é o que a base acrescenta ao que o histórico já
                  sabia. Ela encolhe primeiro: quando falta espaço, o nome do
                  argumento é o que precisa continuar legível. */}
              {candidate.description && (
                <span className="min-w-0 flex-1 truncate text-right text-[11px] text-tyba-text-faint">
                  {candidate.description}
                </span>
              )}
            </button>
          ))}
          {bins.map((candidate) => (
            <button
              key={`bin:${candidate.name}`}
              onMouseDown={(event) => {
                event.preventDefault();
                takeBinary(candidate.name);
              }}
              className="flex w-full items-center gap-2 px-2.5 py-1 text-left font-mono text-[12px] text-tyba-text-muted hover:bg-tyba-text/[.04]"
            >
              <span className="min-w-0 flex-1 truncate">{candidate.name}</span>
              {/* Binário do `$PATH` não recebe marca — é o caso comum, e marcar
                  todos vira ruído. A marca existe para o dono distinguir o que é
                  DELE (o alias que ele escreveu) do que é do sistema. */}
              {candidate.kind !== "path" && (
                <span className="shrink-0 rounded-[3px] border border-tyba-border px-1 text-[9px] text-tyba-text-faint">
                  {candidate.kind === "alias"
                    ? "alias"
                    : candidate.kind === "function"
                      ? "fn"
                      : "builtin"}
                </span>
              )}
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
             Em `app` o custo de resize é zero: entrar em alt-screen JÁ
             redimensiona o painel inteiro (o terminal vai de metade para tudo),
             e as duas coisas acontecem no mesmo commit do React — um resize,
             não dois. Em `off` há um resize, e ele é aceito: trocar de modo é
             ação deliberada e rara, ao contrário de rodar um comando.
             O TEXTO depende do estado, e não só do `program`. Tratar a faixa
             como "sempre app" fazia o modo clássico dizer que um programa tomou
             a tela quando ninguém tomou. */
          <span
            className={`flex min-w-0 flex-1 items-center truncate font-mono text-[11px] text-tyba-text-faint ${
              compact ? "leading-[17px]" : "min-h-[28px]"
            }`}
          >
            {state === "off"
              ? t("commandLineOff")
              : program
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
              setDispensada(false);
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

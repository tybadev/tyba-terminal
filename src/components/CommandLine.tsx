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
import { compactPath } from "../lib/workspaceCwd";
import { elapsedSeconds, isSlow, shellBootStep, type ShellBoot } from "../lib/loading";
import { useNow } from "./ui/loading";
import { toastError } from "../lib/toast";
import {
  boxAcceptsTyping,
  boxIsMounted,
  caretLine,
  clearsDraft,
  controlBytes,
  ghostDoToken,
  ghostFor,
  commandPrefix,
  enterAplicaSugestao,
  lineToken,
  type LineState,
  type MenuItem,
  menuItems,
  menuRows,
  moveSelection,
  pathToken,
  replaceToken,
  SUGGEST_DEBOUNCE_MS,
} from "../lib/commandLine";

const MAX_HEIGHT_PX = 140;
// Identidade fixa: um `[]` literal como default nasceria novo a cada render.
const NO_PENDING: string[] = [];

const PLACEHOLDER_BY_STATE: Record<LineState, string> = {
  own: "commandLinePlaceholder",
  // Em espera, o texto é a etapa real do shell subindo — ver `bootPlaceholder`.
  waiting: "",
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
  /** A branch onde o comando vai rodar, quando se sabe. Ver o rodapé. */
  branch?: string | null;
  /**
   * A caixa nasce com o foco?
   *
   * Só a linha do painel ATIVO, e só quando ela é dona do teclado. Com uma
   * linha por painel, `autoFocus` incondicional fazia cada caixa montada
   * disputar o foco com as outras — e a última a montar ganhava, que não é
   * necessariamente a do painel em que o usuário está.
   */
  focusOnMount?: boolean;
  scope: { cwd: string | null; repoRoot: string | null };
  /**
   * Muda quando a linha deve pegar o foco: volta a ser do TYBA (fim de
   * comando, saída do vim) com o painel ativo, ou o painel volta a ser o ativo.
   */
  focusNonce: number;
  /**
   * Por que a linha não é editável agora. Ela **nunca** desaparece: sumir e
   * voltar a cada comando redimensionava o terminal duas vezes por execução.
   */
  state: LineState;
  /**
   * O shell que está subindo, do core: em `waiting` a linha diz a etapa real
   * ("carregando o ~/.zshrc") em vez de "carregando o shell". `bootHost` é o
   * servidor numa SSH Session — lá o core não sabe o rc.
   */
  boot?: ShellBoot | null;
  bootHost?: string | null;
  /**
   * O shell ainda não reportou o primeiro prompt e a sessão segue viva. O
   * `waiting` também cobre casos fora do boot (broadcast SSH, …); só com isto
   * a linha mostra o filamento e a etapa do boot.
   */
  booting?: boolean;
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
   * Cada painel tem a própria caixa, e trocar o foco não a remonta. Mas ela
   * ainda remonta ao trocar de aba (o painel sai da tela) e ao voltar de um app
   * de tela cheia; o rascunho morria junto — meio comando escrito, uma ida a
   * outra aba, e ele sumia sem aviso.
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
  /**
   * As linhas já enviadas de um comando em continuação, guardadas por SESSÃO
   * pelo mesmo motivo do `draft`: a caixa remonta ao trocar de aba e ao voltar
   * de um app de tela cheia, e sem isto o começo de um `ls \` sumia da tela no
   * meio do comando. Só em memória.
   */
  pending?: string[];
  onPendingChange?: (sessionId: SessionId, lines: string[]) => void;
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
  branch = null,
  focusOnMount = false,
  scope,
  focusNonce,
  state,
  boot = null,
  bootHost = null,
  booting = false,
  inject,
  draft = "",
  onDraftChange,
  pending: pendingSaved = NO_PENDING,
  onPendingChange,
}: Props) {
  // Não é "a linha não é minha", é "a caixa não aceita tecla". A diferença é
  // o `waiting`: a linha ainda não é do TYBA, mas o rascunho pode ser escrito
  // e o Enter fica de pé — ver `boxAcceptsTyping`.
  const waiting = !boxAcceptsTyping(state);
  // App de tela cheia: a linha não desaparece, troca de conteúdo — e encolhe.
  // Encolher só mexe no painel DESTA linha: cada painel tem a sua, e a altura
  // dela sai só da área de conteúdo dele.
  const collapsed = !boxIsMounted(state);
  // `~/…` como no mockup; caminho longo cai no mesmo `…/pai/pasta`.
  const where = cwd ? compactPath(cwd) : null;
  const { t } = useTranslation();
  // A etapa do shell subindo, com o motivo provável passado o normal. O
  // relógio só anda em espera; a referência de início é o spawn (do core) ou,
  // sem ele, a montagem da linha.
  const [mountedAt] = useState(() => Date.now());
  const bootVisible = state === "waiting" && booting;
  const bootNow = useNow(bootVisible);
  const bootStart = boot?.started_at_ms ?? mountedAt;
  const bootStep = shellBootStep(boot, bootHost);
  const bootPlaceholder = !bootVisible
    ? ""
    : isSlow(bootStart, bootNow, "shell", bootHost !== null)
      ? t("shellBootSlow", { s: elapsedSeconds(bootStart, bootNow) })
      : t(bootStep.key, bootStep.params);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState(draft);
  // No fim do rascunho, não no começo. Voltar para um pane e digitar no meio
  // do que já estava escrito seria pior que ter perdido o texto: o comando sai
  // errado e a culpa não é óbvia. Zero só quando não há rascunho.
  const [caret, setCaret] = useState(draft.length);
  const [hits, setHits] = useState<CommandSuggestion[]>([]);
  // O comando do cinza. Vem à parte porque a lista é cronológica e o cinza é
  // frecência — ver `ghost_command` no core.
  const [ghostHit, setGhostHit] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
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
  // Há texto selecionado? Com seleção, quem desenha é o navegador — o cursor
  // de bloco cobriria o realce.
  const [selecting, setSelecting] = useState(false);
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
  // As linhas já enviadas do comando que o shell ainda está esperando fechar.
  //
  // O terminal fica escondido em modo prompt, e a caixa se limpa a cada Enter:
  // sem isto, depois de `ls \` a pessoa digitaria o resto sem ver o começo.
  // Só é DESENHADO em `continuation`; fora dele o valor é resto de submissão
  // antiga e é trocado na próxima.
  const [pending, setPending] = useState<string[]>(pendingSaved);

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
      setGhostHit(null);
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
          setGhostHit(found.ghost);
          setPaths(found.paths);
          setArgs(found.arguments);
          setBins(found.binaries);
          setIndex(-1);
        })
        .catch(() => {
          if (!alive) return;
          setHits([]);
          setGhostHit(null);
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
        if (found.commands.every((hit) => hit.failed)) return;
        setHits(found.commands);
        // O último executado já vem escolhido: é o gesto do shell, ↑ e Enter
        // repetem o comando anterior.
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

  // Uma lista só, na ordem de proximidade da caixa: o item 0 é o que encosta
  // nela. Ver `menuItems` — é lá que o comando que só falhou sai.
  const items = menuItems({ commands: hits, args, bins, paths });
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
  // Na continuação o que se digita é o RESTO de um comando, e histórico e
  // `$PATH` só sabem completar comando inteiro: sugerir `git status` como
  // corpo de um `ls \` é ruído, e um Tab distraído o colaria lá.
  const continuing = state === "continuation";
  const ghost = continuing
    ? ""
    : escolhido.texto ||
      binGhost ||
      ghostFor(text, ghostHit ? [{ command: ghostHit }] : []);
  // A lista se anuncia sozinha quando há o que DESCOBRIR: o comando que existe
  // na máquina (primeiro token) e o que o comando oferece (subcomando, flag,
  // branch). Ela só existia atrás de `Tab` e ninguém aperta Tab para descobrir
  // algo que não sabe que existe; e o cinza sozinho mostra UMA opção, quando a
  // pergunta de quem digita `git ch` é "quais existem?".
  //
  // Anunciada não é escolhida: abre sem seleção, e o Enter roda o que está
  // escrito (ver `enterAplicaSugestao`).
  const anuncia =
    !dispensada && ((cmdPrefix !== null && bins.length > 0) || args.length > 0);
  const showMenu = (menuOpen || anuncia) && items.length > 0 && !continuing;
  const rows = menuRows(items);

  // A lista cresce para CIMA, e quem importa é a linha de baixo — a que
  // encosta na caixa. Sem isto o scroll nasce no topo e o item mais recente,
  // justamente o escolhido, fica escondido sob a borda.
  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const chosen =
      index >= 0
        ? menu.querySelector<HTMLElement>(`[data-index="${index}"]`)
        : null;
    if (!chosen) {
      menu.scrollTop = menu.scrollHeight;
      return;
    }
    // À mão e não `scrollIntoView`: aquele rola também os ancestrais, e a
    // lista de blocos atrás do menu pularia junto.
    const top = chosen.offsetTop;
    const bottom = top + chosen.offsetHeight;
    if (top < menu.scrollTop) menu.scrollTop = top;
    else if (bottom > menu.scrollTop + menu.clientHeight) {
      menu.scrollTop = bottom - menu.clientHeight;
    }
  }, [showMenu, index, items.length]);

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
  useEffect(() => {
    onPendingChange?.(sessionId, pending);
  }, [sessionId, pending, onPendingChange]);

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

  // Cada tipo troca um pedaço diferente da linha: o histórico, a linha
  // inteira; o argumento e o caminho, o token sob o cursor; o binário, só o
  // primeiro token.
  const applyItem = (item: MenuItem) => {
    switch (item.kind) {
      case "history":
      case "snippet":
        apply(item.command, item.command.length);
        setMenuOpen(false);
        return;
      case "argument":
        takeArg(item.value);
        return;
      case "binary":
        takeBinary(item.name);
        return;
      case "path":
        takePath(item.value);
        return;
    }
  };

  // O texto do último render, para quem responde depois de um `await`.
  const textRef = useRef(text);
  textRef.current = text;

  const run = () => {
    const value = text;
    // Em continuação a linha vazia é resposta válida: é ela que fecha um
    // `cat <<EOF` sem corpo ou um `ls \` que não tinha mais nada a dizer.
    if ((!continuing && !value.trim()) || submitting) return;
    setMenuOpen(false);
    setSubmitting(true);
    // A linha só é limpa quando o shell aceitou. Multiline sem bracketed paste
    // é recusado pelo core, e engolir o erro apagaria o que o usuário escreveu
    // sem executar nada.
    // Onde o caret estava no Enter: se o core julgar o comando pela metade, a
    // quebra de linha entra AQUI, não no fim.
    const el = inputRef.current;
    const start = el?.selectionStart ?? value.length;
    const end = el?.selectionEnd ?? start;
    void submitShellLine(sessionId, value)
      .then((outcome) => {
        if (outcome === "incomplete") {
          // Nada foi escrito no PTY. O Enter vira o que o shell faria com
          // ele — pedir mais linha —, só que dentro da caixa, onde o comando
          // inteiro continua editável e sai depois como UMA submissão.
          // Só se o texto ainda for o do Enter: a espera pelo shell pode
          // durar, e o que foi digitado nela não pode ser atropelado.
          if (textRef.current === value) {
            apply(`${value.slice(0, start)}\n${value.slice(end)}`, start + 1);
          }
          return;
        }
        setPending((prev) => (continuing ? [...prev, value] : [value]));
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

    // Num rascunho multilinha, ↑/↓ andam entre as linhas — o gesto do shell
    // (histórico, lista) só vale na borda: ↑ na primeira linha, ↓ na última.
    // Com uma linha só, as duas bordas são a mesma e nada muda. O caret vem do
    // DOM, não do estado: o `caret` do React só é atualizado no `keyup`.
    const edge = caretLine(
      text,
      e.currentTarget.selectionStart ?? text.length,
    );
    if (
      !showMenu &&
      ((e.key === "ArrowUp" && !edge.first) ||
        (e.key === "ArrowDown" && !edge.last))
    ) {
      return;
    }
    // A lista está ACIMA da caixa: ↑ sobe nela (para o mais antigo), ↓ desce
    // de volta. Entrar nela é o gesto que autoriza o Enter a aplicar; do item
    // colado na caixa, ↓ devolve o cursor à linha sem nada escolhido.
    if (showMenu && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      if (e.key === "ArrowDown" && index < 0) {
        // Já na caixa: descer é sair da lista.
        setMenuOpen(false);
        setDispensada(true);
        return;
      }
      const next = moveSelection(index, e.key, items.length);
      setIndex(next);
      setMenuOpen(next >= 0);
      return;
    }
    if (e.key === "ArrowDown" && !showMenu && items.length > 0) {
      e.preventDefault();
      setMenuOpen(true);
      setDispensada(false);
      return;
    }
    // ↑ com a lista fechada é o histórico, como em qualquer shell. Já filtrado
    // pelo que estiver escrito: com a caixa vazia vem o mais recente — e já
    // escolhido, colado na caixa.
    if (e.key === "ArrowUp" && !showMenu) {
      e.preventDefault();
      if (items.length > 0) {
        setIndex(0);
        setMenuOpen(true);
        setDispensada(false);
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
      // Tab é gesto explícito: com um item escolhido, aplica ele. Sem escolha,
      // completa o cinza — com a lista anunciada quase sempre visível, aplicar
      // o item 0 trocaria `git ch` + Tab pela linha inteira de um comando
      // antigo, quando quem aperta Tab quer terminar a PALAVRA.
      if (showMenu && index >= 0 && items[index]) {
        applyItem(items[index]);
        return;
      }
      if (!acceptGhost() && items.length > 0) {
        setMenuOpen(true);
        setDispensada(false);
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
        items[index]
      ) {
        applyItem(items[index]);
        setIndex(-1);
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
    //
    // Agora a faixa é a caixa: superfície levantada (`raised`) com a aresta de
    // luz em cima (`--tyba-edge`) — a mesma receita do bloco marcado. Num
    // terminal BLACKOUT é o degrau que diz "aqui se escreve" sem moldura.
    // `px-4` na linha põe o chevron na coluna do chevron dos blocos.
    //
    // O fundo é o do painel (`--tyba-panel-bg`): no escuro a superfície
    // chapada, no claro o vidro branco a 62% — o mesmo do mockup.
    <div
      className={`relative shrink-0 border-t bg-[var(--tyba-panel-bg)] py-2 transition-colors ${
        focused ? "border-tyba-green/25" : "border-tyba-border"
      }`}
      style={{ boxShadow: "var(--tyba-edge)" }}
    >
      {/* Shell subindo: o filamento corre na aresta de cima enquanto o rc
          carrega. Absoluto, para não mexer na altura da linha (resize). */}
      {bootVisible && (
        <div
          aria-hidden
          className="tyba-flow-line tyba-flow-line--live pointer-events-none absolute inset-x-0 top-0"
        />
      )}
      {showMenu && (
        // Desenhada INVERTIDA (`menuRows`): o item 0 — o último executado, ou o
        // mais provável — é a última linha, colada na caixa. Mais antigo é
        // mais longe dela, e ↑ sobe em direção a ele.
        <div
          ref={menuRef}
          className="absolute bottom-full left-[18px] right-[18px] z-20 mb-1 max-h-56 overflow-y-auto rounded-[6px] border border-tyba-border bg-tyba-raised py-1 shadow-lg"
        >
          {rows.map(({ item, index: i }) => (
            <button
              key={item.key}
              data-index={i}
              onMouseDown={(event) => {
                event.preventDefault();
                applyItem(item);
              }}
              className={`flex w-full items-center gap-2 px-2.5 py-1 text-left font-mono text-[12px] ${
                i === index
                  ? "bg-tyba-green/15 text-tyba-text"
                  : "text-tyba-text-muted hover:bg-tyba-text/[.04]"
              }`}
            >
              <MenuRow item={item} snippetLabel={t("paletteSnippets")} />
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
        className="flex items-start gap-2.5 px-[18px]"
      >
        {/* O chevron é a âncora e o único elemento colorido da linha.
            Ele também é o indicador de foco: some o anel em volta da caixa, e
            quem diz "o teclado é seu" é ele acendendo. Fantasma (mockup) quando a
            linha não é sua, cheio e com brilho quando é — o mesmo verde que o
            header de cada bloco usa para o comando que deu certo. */}
        <span
          aria-hidden
          className={`shrink-0 select-none font-mono transition-all ${
            collapsed
              ? "text-[11px] leading-[17px] text-tyba-text-faint"
              : `pt-1 text-[13px] leading-[20px] ${focused ? "text-tyba-green-fill" : "text-tyba-text-ghost"}`
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
            className="flex min-w-0 flex-1 items-center truncate font-mono text-[11px] leading-[17px] text-tyba-text-faint"
          >
            {state === "off"
              ? t("commandLineOff")
              : program
                ? t("commandLineAppNamed", { program })
                : t("commandLineApp")}
          </span>
        ) : (
        <div className="min-w-0 flex-1">
          {/* O começo do comando que o shell está esperando fechar — ver
              `pending`. Apagado, como texto que já saiu da mão de quem digita.
              Fora do `relative` de baixo: o cinza do ghost é `inset-0` e
              precisa cobrir só a caixa, não estas linhas. */}
          {continuing && pending.length > 0 && (
            <div className="max-h-[100px] overflow-y-auto whitespace-pre-wrap break-words pt-1 font-mono text-[13px] leading-[20px] text-tyba-text-muted">
              {pending.join("\n")}
            </div>
          )}
          <div className="relative">
          {ghost && (
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words py-1 font-mono text-[13px] text-transparent"
            >
              {text}
              <span className="text-tyba-text-ghost">{ghost}</span>
            </div>
          )}
          {/* O cursor de bloco, aceso — só com o foco. Luz é estado: a linha
              que não tem o teclado não brilha. Desenhado numa camada por cima
              da caixa, com o texto transparente até o ponto do caret; a letra
              sob o cursor é redesenhada em cor invertida. */}
          {focused && !waiting && !selecting && (
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words py-1 font-mono text-[13px] text-transparent"
            >
              {text.slice(0, caret)}
              <span
                className="bg-tyba-green text-tyba-text-inverse"
                style={{ boxShadow: "var(--tyba-glow-green)" }}
              >
                {text[caret] && text[caret] !== "\n" ? text[caret] : "\u00a0"}
              </span>
            </div>
          )}
          <textarea
            ref={inputRef}
            autoFocus={focusOnMount}
            rows={1}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={text}
            disabled={waiting}
            placeholder={
              state === "waiting" ? bootPlaceholder : t(PLACEHOLDER_BY_STATE[state])
            }
            onChange={(e) => {
              setText(e.target.value);
              setCaret(e.target.selectionStart ?? 0);
              setMenuOpen(false);
              setDispensada(false);
            }}
            onKeyDown={onKeyDown}
            onKeyUp={() => setCaret(inputRef.current?.selectionStart ?? 0)}
            onClick={() => setCaret(inputRef.current?.selectionStart ?? 0)}
            // Seta segurada só solta o `keyup` no fim: sem o `select`, o
            // cursor de bloco ficaria parado enquanto o caret anda.
            onSelect={(e) => {
              const el = e.currentTarget;
              setCaret(el.selectionStart ?? 0);
              setSelecting(el.selectionStart !== el.selectionEnd);
            }}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            className={`max-h-[140px] min-h-[28px] w-full resize-none border-0 bg-transparent py-1 font-mono text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-ghost ${
              focused && !waiting && !selecting ? "caret-transparent" : ""
            }`}
          />
          </div>
        </div>
        )}

        {/* Onde o comando vai rodar.
            Encurtado com a MESMA regra do header de cada bloco (`…/pai/pasta`),
            e não só o nome da pasta: duas pastas `src` em repositórios
            diferentes são indistinguíveis pelo basename, e é exatamente na hora
            de apertar Enter que essa distinção importa.
            A branch vem antes, em azul (a cor do git): com dois painéis em
            repositórios diferentes, a da status bar é só a do painel ativo. */}
        {(where || branch) && !collapsed && (
          <span className="flex shrink-0 items-center gap-2 truncate pt-1.5 font-mono text-[11px] leading-[17px] text-tyba-text-faint">
            {branch && <span className="text-tyba-blue">{branch}</span>}
            {where && <span title={cwd ?? undefined}>{where}</span>}
          </span>
        )}
      </div>
    </div>
  );
}

const BADGE =
  "shrink-0 rounded-[3px] border border-tyba-border px-1 text-[9px] text-tyba-text-faint";

/** O miolo de uma linha da lista, conforme de onde o item veio. */
function MenuRow({ item, snippetLabel }: { item: MenuItem; snippetLabel: string }) {
  switch (item.kind) {
    case "history":
      return <span className="min-w-0 flex-1 truncate">{item.command}</span>;
    case "snippet":
      return (
        <>
          <span className="min-w-0 flex-1 truncate">{item.command}</span>
          <span className={BADGE}>{item.label ?? snippetLabel}</span>
        </>
      );
    case "argument":
      return (
        <>
          <span className="shrink-0">{item.value}</span>
          {/* A descrição é o que a base acrescenta ao que o histórico já
              sabia. Ela encolhe primeiro: quando falta espaço, o nome do
              argumento é o que precisa continuar legível. */}
          {item.description && (
            <span className="min-w-0 flex-1 truncate text-right text-[11px] text-tyba-text-faint">
              {item.description}
            </span>
          )}
        </>
      );
    case "binary":
      return (
        <>
          <span className="min-w-0 flex-1 truncate">{item.name}</span>
          {/* Binário do `$PATH` não recebe marca — é o caso comum, e marcar
              todos vira ruído. A marca existe para o dono distinguir o que é
              DELE (o alias que ele escreveu) do que é do sistema. */}
          {item.binKind !== "path" && (
            <span className={BADGE}>
              {item.binKind === "alias"
                ? "alias"
                : item.binKind === "function"
                  ? "fn"
                  : "builtin"}
            </span>
          )}
        </>
      );
    case "path":
      return <span className="min-w-0 flex-1 truncate">{item.value}</span>;
  }
}

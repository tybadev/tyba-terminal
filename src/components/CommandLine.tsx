import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretRight, GitBranch } from "@phosphor-icons/react";

import {
  submitShellLine,
  suggestLine,
  writeControl,
  type CommandSuggestion,
  type SessionId,
} from "../lib/ipc";
import { toastError } from "../lib/toast";
import {
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
  running: "commandLineRunning",
  app: "commandLineApp",
  off: "commandLineOff",
};

interface Props {
  sessionId: SessionId;
  cwd: string | null;
  branch: string | null;
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

function baseName(path: string | null): string {
  if (!path) return "";
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1) || "/";
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
  branch,
  scope,
  focusNonce,
  state,
  inject,
}: Props) {
  const waiting = state !== "own";
  const { t } = useTranslation();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState("");
  const [caret, setCaret] = useState(0);
  const [hits, setHits] = useState<CommandSuggestion[]>([]);
  const [index, setIndex] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);
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
      el.style.height = "auto";
      el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT_PX)}px`;
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

  const apply = (next: string, nextCaret: number) => {
    setText(next);
    setCaret(nextCaret);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.setSelectionRange(nextCaret, nextCaret);
      resize();
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

  const dir = baseName(cwd);

  return (
    <div className="relative shrink-0 border-t border-tyba-border bg-tyba-sunken px-3 py-2">
      {showMenu && (
        <div className="absolute bottom-full left-3 right-3 z-20 mb-1 max-h-56 overflow-y-auto rounded-[6px] border border-tyba-border bg-tyba-raised py-1 shadow-lg">
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

      <div className="flex items-start gap-2">
        {/* O PS1 saiu da tela; o que ele dizia (onde estou, em que branch) não
            pode sumir junto. */}
        <div className="flex h-7 shrink-0 items-center gap-1.5 font-mono text-[12px] text-tyba-text-faint">
          {dir && <span className="max-w-40 truncate">{dir}</span>}
          {branch && (
            <span className="flex items-center gap-0.5 truncate">
              <GitBranch size={11} />
              <span className="max-w-32 truncate">{branch}</span>
            </span>
          )}
          <CaretRight size={12} weight="bold" className="text-tyba-green" />
        </div>

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
              resize();
            }}
            onKeyDown={onKeyDown}
            onKeyUp={() => setCaret(inputRef.current?.selectionStart ?? 0)}
            onClick={() => setCaret(inputRef.current?.selectionStart ?? 0)}
            className="max-h-[140px] min-h-[28px] w-full resize-none border-0 bg-transparent py-1 font-mono text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </div>
      </div>
    </div>
  );
}

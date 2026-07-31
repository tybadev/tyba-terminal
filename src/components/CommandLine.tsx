import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretRight, GitBranch } from "@phosphor-icons/react";

import {
  submitShellLine,
  suggestCommands,
  writeControl,
  type CommandSuggestion,
  type SessionId,
} from "../lib/ipc";
import {
  clearsDraft,
  controlBytes,
  ghostFor,
  SUGGEST_DEBOUNCE_MS,
} from "../lib/commandLine";

const MAX_HEIGHT_PX = 140;

interface Props {
  sessionId: SessionId;
  cwd: string | null;
  branch: string | null;
  scope: { cwd: string | null; repoRoot: string | null };
  /** Muda quando a linha volta a ser do TYBA (fim de comando, saída do vim). */
  focusNonce: number;
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
}: Props) {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState("");
  const [caret, setCaret] = useState(0);
  const [hits, setHits] = useState<CommandSuggestion[]>([]);
  const [index, setIndex] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);

  const seenNonce = useRef(focusNonce);
  useEffect(() => {
    if (focusNonce === seenNonce.current) return;
    seenNonce.current = focusNonce;
    inputRef.current?.focus();
  }, [focusNonce]);

  // Sem nada digitado não há o que sugerir: lista aberta com a linha vazia é
  // ruído em cima da saída do último comando.
  useEffect(() => {
    if (!text.trim()) {
      setHits([]);
      setMenuOpen(false);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      void suggestCommands(text, scope.cwd, scope.repoRoot)
        .then((found) => {
          if (!alive) return;
          setHits(found);
          setIndex(0);
        })
        .catch(() => {
          if (alive) setHits([]);
        });
    }, SUGGEST_DEBOUNCE_MS);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [text, scope.cwd, scope.repoRoot]);

  // Comando que só falhou completa no cinza quando é prefixo do que se está
  // digitando, mas nunca é oferecido como item — devolver `lljh` numa lista é
  // sugerir o próprio erro de digitação.
  const listed = hits.filter((hit) => !hit.failed);
  const ghost = ghostFor(text, hits);
  const showMenu = menuOpen && listed.length > 0;

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
    apply(text + ghost, text.length + ghost.length);
    return true;
  };

  const run = () => {
    const value = text;
    if (!value.trim()) return;
    apply("", 0);
    setHits([]);
    setMenuOpen(false);
    void submitShellLine(sessionId, value).catch(() => {});
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
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
      setIndex((prev) => (prev + delta + listed.length) % listed.length);
      return;
    }
    if (e.key === "ArrowDown" && !showMenu && listed.length > 0) {
      e.preventDefault();
      setMenuOpen(true);
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
      if (!acceptGhost() && listed.length > 0) setMenuOpen(true);
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
      if (showMenu) {
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
            placeholder={t("commandLinePlaceholder")}
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

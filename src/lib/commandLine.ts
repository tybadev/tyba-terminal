import type { SessionCommand, SessionKind } from "./ipc";

export const PROMPT_MODE_PREF_KEY = "pref.promptMode";

/**
 * Quem é dono do teclado neste instante.
 *
 * `terminal` não é fallback de UX, é correção: `ssh`, `read`, `psql` e o prompt
 * de senha do `sudo` leem stdin DURANTE o comando. Se a linha do TYBA engolir
 * essas teclas, o usuário digita a senha num campo que não vai a lugar nenhum.
 */
export type KeyboardOwner = "terminal" | "tybaLine";

export interface OwnerInput {
  /** Preferência ligada e o shell iniciado em modo prompt do TYBA. */
  promptMode: boolean;
  kind: SessionKind | undefined;
  /** `term.buffer.active.type === "alternate"` — vim, htop, less. */
  altScreen: boolean;
  /** Entre `133;C` e `133;D`. */
  command: SessionCommand | undefined;
  /** Sem `133;A` não há como saber que o shell está no prompt. */
  integrated: boolean;
}

export function keyboardOwner({
  promptMode,
  kind,
  altScreen,
  command,
  integrated,
}: OwnerInput): KeyboardOwner {
  if (!promptMode || !integrated) return "terminal";
  if (kind?.type !== "shell") return "terminal";
  if (altScreen) return "terminal";
  if (command?.running) return "terminal";
  return "tybaLine";
}

/**
 * Teclas que a linha do TYBA nunca consome: são sinais para o processo, não
 * texto. Ctrl+C também limpa a caixa — o usuário espera perder o rascunho.
 */
const CONTROL_KEYS: Record<string, string> = {
  c: "\x03",
  d: "\x04",
  z: "\x1a",
  "\\": "\x1c",
};

export interface ControlChord {
  key: string;
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
}

export function controlBytes(chord: ControlChord): string | null {
  if (!chord.ctrl || chord.meta || chord.alt) return null;
  return CONTROL_KEYS[chord.key.toLowerCase()] ?? null;
}

export function clearsDraft(chord: ControlChord): boolean {
  return controlBytes(chord) === "\x03";
}

export const SUGGEST_DEBOUNCE_MS = 70;

export interface PathToken {
  /** Índice onde o token começa, para trocá-lo sem tocar no resto da linha. */
  start: number;
  value: string;
}

const PATHISH = /^(\.{1,2}\/|~|\/)/;

/**
 * O token que deve ser completado como caminho, ou `null`.
 *
 * A primeira palavra da linha é posição de COMANDO, não de arquivo: num
 * diretório com `teste/`, completar `te` para `teste/` transformaria o começo de
 * `test` numa pasta. Só vale como caminho se for argumento, ou se o próprio
 * token já disser que é caminho (`./`, `../`, `~`, `/`, ou contém barra).
 */
export function pathToken(text: string, caret: number): PathToken | null {
  const before = text.slice(0, caret);
  const match = /[^\s]*$/.exec(before);
  if (!match) return null;
  const value = match[0];
  if (!value) return null;
  const start = before.length - value.length;
  const isFirstWord = before.slice(0, start).trim().length === 0;
  if (isFirstWord && !PATHISH.test(value) && !value.includes("/")) return null;
  return { start, value };
}

/** Troca só o token do caminho, preservando o resto da linha. */
export function replaceToken(
  text: string,
  token: PathToken,
  completion: string,
): { text: string; caret: number } {
  const next =
    text.slice(0, token.start) +
    completion +
    text.slice(token.start + token.value.length);
  return { text: next, caret: token.start + completion.length };
}

export interface Suggestion {
  command: string;
}

/**
 * O resto do comando que aparece em cinza. Só a primeira sugestão que
 * **começa** com o que já foi digitado serve — ghost text que não é prefixo
 * mentiria sobre o que o `→` vai completar.
 */
export function ghostFor(text: string, hits: Suggestion[]): string {
  if (!text.trim()) return "";
  const hit = hits.find(
    (candidate) =>
      candidate.command.length > text.length &&
      candidate.command.startsWith(text),
  );
  return hit ? hit.command.slice(text.length) : "";
}

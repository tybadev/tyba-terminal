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

/**
 * Por que a linha não é editável agora — ela nunca desaparece.
 *
 * Sumir e voltar a cada comando redimensionava o terminal duas vezes por
 * execução, e o `vim` reabria com outra altura.
 */
export type LineState = "own" | "waiting" | "running" | "app" | "off";

/** O shell respondeu que NÃO está em modo prompt, mas o usuário quer estar. */
export function isOff(input: OwnerInput & { reported: boolean | undefined }) {
  return input.reported === false;
}

export function lineState(
  input: OwnerInput & { reported?: boolean | undefined },
): LineState {
  if (keyboardOwner(input) === "tybaLine") return "own";
  // Desligado de propósito (ou por engano num ⌘⇧L a mais): a linha continua na
  // tela dizendo isso. Ela sumir sem explicação foi o que fez o modo clássico
  // parecer defeito.
  if (input.reported === false) return "off";
  if (input.altScreen) return "app";
  if (input.command?.running) return "running";
  return "waiting";
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

/** As quatro setas, pelo `key` do evento de teclado. */
const ARROW_KEYS = new Set([
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
]);

export function isArrowKey(key: string): boolean {
  return ARROW_KEYS.has(key);
}

export interface ArrowInput {
  /** Entre `133;C` e `133;D`. Fora disso a linha do TYBA já é dona do teclado. */
  running: boolean;
  /**
   * `ECHO` do termios: o tty entrega LINHAS, não teclas.
   *
   * Ligado, o driver só devolve a linha ao dar Enter e trata apenas
   * backspace/kill — a seta entra como byte literal que nenhum leitor de linha
   * interpreta, e ainda é ecoada de volta.
   */
  lineEcho: boolean;
  /** vim, htop, less: a tela é do programa e as setas também. */
  altScreen: boolean;
}

/**
 * A seta morre aqui em vez de ir para o PTY?
 *
 * O caso: com um comando rodando, apertar seta escrevia `^[[A` na saída — e a
 * saída vira bloco gravado no SQLite e no markdown do copiar. Some da tela ao
 * limpar, não do disco.
 *
 * Só quando o tty está em modo linha, e essa condição é a causa, não um proxy
 * dela: é exatamente nesse estado que a seta não serve a ninguém e é ecoada.
 * Em raw, quem lê tecla a tecla precisa dela — o menu do `npm create`, que é
 * canônico ao perguntar `Ok to proceed? (y)` e vira raw ao abrir a lista, no
 * mesmo comando.
 *
 * Não vale para o resto do teclado: o `y` daquele prompt também é canônico com
 * eco, e engoli-lo impediria responder. Ver {@link KeyboardOwner}.
 */
export function swallowsArrow({
  running,
  lineEcho,
  altScreen,
}: ArrowInput): boolean {
  if (!running || altScreen) return false;
  return lineEcho;
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
 * O token sob o cursor e o que vem antes dele na linha.
 *
 * O prefixo é o que dá contexto: `git ` completa subcomando, `cargo test `
 * completa flag. Sem ele a sugestão seria "qualquer palavra que já digitei",
 * que é trabalho do histórico.
 */
export interface LineToken {
  start: number;
  value: string;
  prefix: string;
}

export function lineToken(text: string, caret: number): LineToken | null {
  const before = text.slice(0, caret);
  const match = /[^\s]*$/.exec(before);
  if (!match) return null;
  const value = match[0];
  const start = before.length - value.length;
  const prefix = before.slice(0, start);
  if (!prefix.trim()) return null;
  return { start, value, prefix };
}

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

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

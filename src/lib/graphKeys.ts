// Atalhos do canvas (canvas-5, canvas-9 e a paleta). Puro: a tecla e quem
// tem o foco entram, a ação sai. O canvas só age quando o teclado não é de um
// campo, do terminal de um nó (`.nokey`) nem de um diálogo.

export type CanvasKey =
  "undo" | "redo" | "duplicate" | "copy" | "paste" | "selectAll" | "zoomActual" | "fit" | "palette";

type KeyLike = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">;

/**
 * A ação do canvas para a tecla, ou `null`. O modificador é ⌘ no mac e Ctrl
 * fora dele; `/` sozinho também abre a paleta.
 */
export function canvasKey(e: KeyLike, isMac: boolean): CanvasKey | null {
  const mod = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (e.altKey) return null;
  if (!mod) {
    return e.key === "/" && !e.metaKey && !e.ctrlKey && !e.shiftKey ? "palette" : null;
  }
  const key = e.key.toLowerCase();
  if (e.shiftKey) return key === "z" ? "redo" : null;
  switch (key) {
    case "z":
      return "undo";
    case "d":
      return "duplicate";
    case "c":
      return "copy";
    case "v":
      return "paste";
    case "a":
      return "selectAll";
    case "0":
      return "zoomActual";
    case "1":
      return "fit";
    case "k":
      return "palette";
    default:
      return null;
  }
}

interface ElementLike {
  tagName: string;
  isContentEditable?: boolean;
  closest(selector: string): unknown;
}

/**
 * O teclado é de outro dono (campo de texto, terminal do nó, diálogo, menu
 * aberto)? Então o canvas não pega a tecla — ⌘Z desfaz o texto do campo,
 * ⌘C copia do terminal.
 */
export function keyOwnedElsewhere(target: ElementLike | null): boolean {
  if (!target) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (target.isContentEditable) return true;
  return Boolean(target.closest('.nokey, [role="dialog"], [role="menu"], [data-maestro-sheet]'));
}

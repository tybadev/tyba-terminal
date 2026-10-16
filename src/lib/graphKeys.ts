// Atalhos do canvas (canvas-5, canvas-9 e a paleta). Puro: a tecla e quem
// tem o foco entram, a ação sai. O canvas só age quando o teclado não é de um
// campo, do terminal de um nó (`.nokey`) nem de um diálogo.

export type CanvasKey =
  | "undo"
  | "redo"
  | "duplicate"
  | "copy"
  | "paste"
  | "selectAll"
  | "zoomActual"
  | "fit"
  | "palette"
  | "focusTerminal";

type KeyLike = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">;

/**
 * A ação do canvas para a tecla, ou `null`. O modificador é ⌘ no mac e Ctrl
 * fora dele; `/` sozinho também abre a paleta e Enter sozinho foca o
 * terminal do nó selecionado.
 */
export function canvasKey(e: KeyLike, isMac: boolean): CanvasKey | null {
  const mod = isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (e.altKey) return null;
  if (!mod) {
    if (e.metaKey || e.ctrlKey || e.shiftKey) return null;
    if (e.key === "/") return "palette";
    return e.key === "Enter" ? "focusTerminal" : null;
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
 * aberto, a página do nó de navegador)? Então o canvas não pega a tecla — ⌘Z
 * desfaz o texto do campo, ⌘C copia do terminal.
 */
export function keyOwnedElsewhere(target: ElementLike | null): boolean {
  if (!target) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  // Tecla dentro da página vai para o documento dela, não para este; o
  // iframe em foco só aparece aqui se alguma chegar mesmo assim.
  if (tag === "IFRAME") return true;
  if (target.isContentEditable) return true;
  return Boolean(target.closest('.nokey, [role="dialog"], [role="menu"], [data-orbit-sheet]'));
}

/**
 * A sessão cujo terminal o Enter foca: a do nó selecionado, se for um só e
 * tiver sessão. Selecionar não foca o terminal — o foco vem do clique nele
 * ou deste Enter —, senão ⌘A focaria todos e ⌘D/⌘C nunca valeriam num nó
 * rodando (o teclado já seria do terminal).
 */
export function enterTerminal(
  selected: readonly { id: string }[],
  sessionOf: (id: string) => string | null,
): string | null {
  return selected.length === 1 ? sessionOf(selected[0].id) : null;
}

/**
 * O dono está usando a página do nó de navegador: o foco do app foi para o
 * iframe dele (a janela do app perdeu o foco e o elemento ativo é o frame).
 * Estado de tela, efêmero: clicar fora devolve.
 */
export function frameInUse(active: unknown, frame: unknown): boolean {
  return frame != null && active === frame;
}

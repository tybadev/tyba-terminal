import {
  ACTION_LABEL_KEYS,
  isTerminalAction,
  type Bindings,
  type KeyAction,
} from "./keys";

export type MenuItemSpec =
  | { kind: "separator" }
  | { kind: "predefined"; name: string; label?: string }
  | { kind: "action"; id: string; label: string; accelerator?: string };

export interface MenuSubmenuSpec {
  label: string;
  items: MenuItemSpec[];
}

export interface MenuSpec {
  submenus: MenuSubmenuSpec[];
}

export const MENU_EXTRA_IDS = [
  "menu:shortcuts",
  "menu:checkUpdates",
  "menu:docs",
  "menu:changelog",
  "menu:issues",
] as const;

export type MenuExtraId = (typeof MENU_EXTRA_IDS)[number];

export function isMenuExtraId(id: string): id is MenuExtraId {
  return (MENU_EXTRA_IDS as readonly string[]).includes(id);
}

const MODIFIERS: Record<string, string> = {
  meta: "Cmd",
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
};

/**
 * Converte o combo interno (`meta+shift+t`) para o formato do muda
 * (`Cmd+Shift+T`). Devolve `null` para combo que o parser do muda recusaria —
 * acelerador inválido derrubaria o item no core.
 */
export function toAccelerator(combo: string): string | null {
  const parts = combo.split("+").filter((part) => part.length > 0);
  if (parts.length < 2) return null;
  const key = parts[parts.length - 1];
  const modifiers = parts.slice(0, -1);
  if (key in MODIFIERS) return null;
  if (!modifiers.every((part) => part in MODIFIERS)) return null;
  const named = modifiers.map((part) => MODIFIERS[part]);
  return [...named, key.length === 1 ? key.toUpperCase() : key].join("+");
}

type Translate = (key: string) => string;

/**
 * Ação de terminal nunca propõe acelerador: no macOS o AppKit o consome antes
 * do webview, e ⌘C de menu desligaria o copiar dentro do xterm. O core repete a
 * recusa em `menu::sanitize_accelerator` — esta é a primeira camada, não a
 * única.
 */
function action(
  id: KeyAction,
  t: Translate,
  bindings: Bindings,
): MenuItemSpec {
  const accelerator = isTerminalAction(id)
    ? null
    : toAccelerator(bindings[id]);
  return {
    kind: "action",
    id,
    label: t(ACTION_LABEL_KEYS[id]),
    ...(accelerator ? { accelerator } : {}),
  };
}

function extra(id: MenuExtraId, label: string): MenuItemSpec {
  return { kind: "action", id, label };
}

/**
 * Espelho do menu nativo. Rótulo de ação sai de `ACTION_LABEL_KEYS` e o
 * acelerador da tabela de atalhos do usuário — as duas fontes que o painel de
 * atalhos e a paleta já usam, para o menu nunca discordar delas.
 *
 * O core decide quais ids podem carregar acelerador (`menu::sanitize_accelerator`):
 * aqui pode-se propor, lá é que se aceita.
 */
export function buildMenuSpec(t: Translate, bindings: Bindings): MenuSpec {
  return {
    submenus: [
      {
        label: "Tyba",
        items: [
          { kind: "predefined", name: "about", label: t("menuAbout") },
          extra("menu:checkUpdates", t("menuCheckUpdates")),
          { kind: "separator" },
          action("settings", t, bindings),
          extra("menu:shortcuts", t("shortcuts")),
          { kind: "separator" },
          { kind: "predefined", name: "services", label: t("menuServices") },
          { kind: "separator" },
          { kind: "predefined", name: "hide", label: t("menuHide") },
          {
            kind: "predefined",
            name: "hide_others",
            label: t("menuHideOthers"),
          },
          { kind: "predefined", name: "show_all", label: t("menuShowAll") },
          { kind: "separator" },
          { kind: "predefined", name: "quit", label: t("menuQuit") },
        ],
      },
      {
        label: t("menuFile"),
        items: [
          action("newSession", t, bindings),
          action("newWorktreeSession", t, bindings),
          action("newTab", t, bindings),
          action("newWindow", t, bindings),
          { kind: "separator" },
          action("openFolder", t, bindings),
          { kind: "separator" },
          action("closePane", t, bindings),
          {
            kind: "predefined",
            name: "close_window",
            label: t("menuCloseWindow"),
          },
        ],
      },
      {
        label: t("menuEdit"),
        items: [
          { kind: "predefined", name: "undo", label: t("menuUndo") },
          { kind: "predefined", name: "redo", label: t("menuRedo") },
          { kind: "separator" },
          { kind: "predefined", name: "cut", label: t("menuCut") },
          { kind: "predefined", name: "copy", label: t("menuCopy") },
          { kind: "predefined", name: "paste", label: t("menuPaste") },
          {
            kind: "predefined",
            name: "select_all",
            label: t("menuSelectAll"),
          },
          { kind: "separator" },
          action("search", t, bindings),
        ],
      },
      {
        label: t("menuView"),
        items: [
          action("paletteActions", t, bindings),
          action("paletteSessions", t, bindings),
          action("filesFinder", t, bindings),
          { kind: "separator" },
          action("panel", t, bindings),
          action("files", t, bindings),
        ],
      },
      {
        label: t("menuSession"),
        items: [
          action("splitRight", t, bindings),
          action("splitDown", t, bindings),
          { kind: "separator" },
          action("nextPane", t, bindings),
          action("prevTab", t, bindings),
          action("nextTab", t, bindings),
        ],
      },
      {
        label: t("menuWindow"),
        items: [
          { kind: "predefined", name: "minimize", label: t("menuMinimize") },
          { kind: "predefined", name: "maximize", label: t("menuZoom") },
          {
            kind: "predefined",
            name: "fullscreen",
            label: t("menuFullscreen"),
          },
          { kind: "separator" },
          {
            kind: "predefined",
            name: "bring_all_to_front",
            label: t("menuBringAllToFront"),
          },
        ],
      },
      {
        label: t("menuHelp"),
        items: [
          extra("menu:docs", t("aboutDocs")),
          extra("menu:changelog", t("aboutChangelog")),
          extra("menu:issues", t("menuReportIssue")),
        ],
      },
    ],
  };
}

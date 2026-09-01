import { describe, expect, it } from "bun:test";

import { IS_MAC } from "./platform";
import {
  DEFAULT_BINDINGS,
  KEY_ACTIONS,
  MAC_BINDINGS,
  OPEN_LATEST_SESSION_COMBO,
  PANE_RESIZE_COMBO_PREFIX,
  PC_BINDINGS,
  comboKeys,
  comboOf,
  formatCombo,
  isBoundCombo,
  isPaneResizeChord,
  isTabDigitChord,
  parseBindings,
  tabDigitCombo,
  type Bindings,
} from "./keys";

const MODIFIERS = ["meta", "ctrl", "alt", "shift"];

const TABLES: [string, Bindings][] = [
  ["macOS", MAC_BINDINGS],
  ["Linux/Windows", PC_BINDINGS],
];

function chord(
  init: Partial<
    Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">
  >,
): KeyboardEvent {
  return {
    key: "a",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...init,
  } as KeyboardEvent;
}

describe.each(TABLES)("tabela de %s", (_name, table) => {
  it("cobre toda ação sem deixar buraco", () => {
    for (const action of KEY_ACTIONS) {
      expect(table[action]).toBeTruthy();
    }
  });

  it("não repete combo entre duas ações", () => {
    const seen = new Map<string, string>();
    for (const action of KEY_ACTIONS) {
      const combo = table[action];
      expect(seen.get(combo) ?? action).toBe(action);
      seen.set(combo, action);
    }
  });

  it("usa a ordem canônica de modificadores do comboOf", () => {
    for (const action of KEY_ACTIONS) {
      const parts = table[action].split("+");
      const mods = parts.slice(0, -1);
      expect(mods).toEqual(MODIFIERS.filter((m) => mods.includes(m)));
    }
  });

  it("todo default tem pelo menos um modificador", () => {
    for (const action of KEY_ACTIONS) {
      expect(table[action].split("+").length).toBeGreaterThan(1);
    }
  });
});

describe("tabela do Linux/Windows", () => {
  it("nunca usa meta: fora do macOS meta é a tecla Super", () => {
    for (const action of KEY_ACTIONS) {
      expect(PC_BINDINGS[action].split("+")).not.toContain("meta");
    }
  });

  it("não rouba Ctrl+letra do shell (Ctrl+B tmux, Ctrl+D EOF, Ctrl+W kill-word)", () => {
    for (const action of KEY_ACTIONS) {
      const parts = PC_BINDINGS[action].split("+");
      const key = parts[parts.length - 1];
      const mods = parts.slice(0, -1);
      const bareCtrlLetter =
        mods.length === 1 && mods[0] === "ctrl" && /^[a-z]$/.test(key);
      expect(bareCtrlLetter).toBe(false);
    }
  });
});

describe("tabela do macOS", () => {
  it("todo default passa por Cmd", () => {
    for (const action of KEY_ACTIONS) {
      expect(MAC_BINDINGS[action].split("+")).toContain("meta");
    }
  });
});

describe("DEFAULT_BINDINGS", () => {
  it("é a tabela da plataforma em que o app roda", () => {
    expect(DEFAULT_BINDINGS).toEqual(IS_MAC ? MAC_BINDINGS : PC_BINDINGS);
  });
});

describe("comboKeys", () => {
  it("desenha o modificador da plataforma certa", () => {
    expect(comboKeys("ctrl+shift+p")).toEqual(
      IS_MAC ? ["⌃", "⇧", "P"] : ["Ctrl", "Shift", "P"],
    );
  });

  it("mantém as setas legíveis nas duas plataformas", () => {
    expect(comboKeys("ctrl+alt+arrowleft").at(-1)).toBe("←");
  });
});

describe("formatCombo", () => {
  it("separa com + fora do macOS e cola no macOS", () => {
    expect(formatCombo("ctrl+shift+p")).toBe(IS_MAC ? "⌃⇧P" : "Ctrl+Shift+P");
  });
});

describe("isTabDigitChord", () => {
  it("aceita o modificador da plataforma", () => {
    const e = IS_MAC
      ? chord({ key: "1", metaKey: true })
      : chord({ key: "1", altKey: true });
    expect(isTabDigitChord(e)).toBe(true);
  });

  it("recusa o modificador da outra plataforma", () => {
    const e = IS_MAC
      ? chord({ key: "1", altKey: true })
      : chord({ key: "1", metaKey: true });
    expect(isTabDigitChord(e)).toBe(false);
  });

  it("recusa quando há modificador sobrando", () => {
    const e = IS_MAC
      ? chord({ key: "1", metaKey: true, shiftKey: true })
      : chord({ key: "1", altKey: true, shiftKey: true });
    expect(isTabDigitChord(e)).toBe(false);
  });
});

describe("isPaneResizeChord", () => {
  it("casa com o acorde da plataforma", () => {
    const e = IS_MAC
      ? chord({ key: "ArrowLeft", metaKey: true, ctrlKey: true })
      : chord({
          key: "ArrowLeft",
          ctrlKey: true,
          altKey: true,
          shiftKey: true,
        });
    expect(isPaneResizeChord(e)).toBe(true);
  });

  it("não engole o acorde de foco de pane", () => {
    const e = IS_MAC
      ? chord({ key: "ArrowLeft", metaKey: true, altKey: true })
      : chord({ key: "ArrowLeft", ctrlKey: true, altKey: true });
    expect(isPaneResizeChord(e)).toBe(false);
  });

  it("o prefixo de resize não é prefixo do default de foco de pane", () => {
    for (const [, table] of TABLES) {
      expect(table.paneLeft.startsWith(PANE_RESIZE_COMBO_PREFIX)).toBe(false);
    }
  });
});

describe("tabDigitCombo", () => {
  it("rende o rótulo da plataforma", () => {
    expect(formatCombo(tabDigitCombo(1))).toBe(IS_MAC ? "⌘1" : "Alt+1");
  });
});

describe("OPEN_LATEST_SESSION_COMBO", () => {
  it("não colide com nenhum atalho rebindável da plataforma", () => {
    for (const action of KEY_ACTIONS) {
      expect(DEFAULT_BINDINGS[action]).not.toBe(OPEN_LATEST_SESSION_COMBO);
    }
  });
});

describe("parseBindings", () => {
  it("cai no default quando não há nada salvo", () => {
    expect(parseBindings(null)).toEqual(DEFAULT_BINDINGS);
  });

  it("preserva o que o usuário gravou", () => {
    expect(parseBindings(JSON.stringify({ panel: "ctrl+alt+z" })).panel).toBe(
      "ctrl+alt+z",
    );
  });

  it("ignora lixo e mantém o resto", () => {
    const parsed = parseBindings(
      JSON.stringify({ panel: 42, newTab: "ctrl+shift+y" }),
    );
    expect(parsed.panel).toBe(DEFAULT_BINDINGS.panel);
    expect(parsed.newTab).toBe("ctrl+shift+y");
  });
});

describe("isBoundCombo", () => {
  // B2: TerminalView só engole Ctrl+Alt+Seta/Ctrl+Shift+Seta (pane-nav,
  // prevSession etc.) pro PTY quando NENHUM binding do app casa com o
  // combo — a checagem tem que valer pra QUALQUER atalho que use seta como
  // tecla-base, não só pane-nav (paneLeft/Right/Up/Down usam Ctrl+Alt+Seta
  // no PC/Meta+Alt+Seta no mac; prevSession/nextSession usam
  // Ctrl+Shift+Seta/Meta+Shift+Seta) — todos sofriam do mesmo bug.
  it("reconhece um combo que bate com algum binding da tabela ativa", () => {
    expect(isBoundCombo(DEFAULT_BINDINGS, DEFAULT_BINDINGS.paneDown)).toBe(
      true,
    );
  });

  it("não reconhece combo nenhum, nunca gravado na tabela", () => {
    expect(isBoundCombo(DEFAULT_BINDINGS, "ctrl+alt+shift+z")).toBe(false);
  });

  it("combo nulo (comboOf sem modificador) nunca é atalho do app", () => {
    expect(isBoundCombo(DEFAULT_BINDINGS, null)).toBe(false);
  });

  it("Ctrl+Alt+ArrowDown (pane-nav) é reconhecido via comboOf real", () => {
    const combo = comboOf(
      chord({ key: "ArrowDown", ctrlKey: true, altKey: true }),
    );
    expect(isBoundCombo(PC_BINDINGS, combo)).toBe(true);
  });

  it("seta pura, sem modificador, não é atalho do app (segue pro PTY)", () => {
    const combo = comboOf(chord({ key: "ArrowDown" }));
    expect(isBoundCombo(PC_BINDINGS, combo)).toBe(false);
  });
});

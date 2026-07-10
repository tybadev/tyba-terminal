import { describe, expect, test } from "bun:test";

import {
  DEFAULT_RICH_INPUT,
  atQuery,
  enterAction,
  insertToken,
  parseRichInputPref,
  shouldShowRichInput,
  toRelPath,
} from "./richInput";

describe("parseRichInputPref", () => {
  test("null e JSON inválido caem no default", () => {
    expect(parseRichInputPref(null)).toEqual(DEFAULT_RICH_INPUT);
    expect(parseRichInputPref("{broken")).toEqual(DEFAULT_RICH_INPUT);
  });

  test("versão futura cai no default", () => {
    expect(parseRichInputPref('{"version":99,"autoShow":false}')).toEqual(
      DEFAULT_RICH_INPUT,
    );
  });

  test("campos válidos sobrescrevem e inválidos mantêm default", () => {
    const pref = parseRichInputPref(
      '{"version":1,"submitWithCtrlEnter":true,"autoShow":"nope","agentRegex":"^aider"}',
    );
    expect(pref.submitWithCtrlEnter).toBe(true);
    expect(pref.autoShow).toBe(DEFAULT_RICH_INPUT.autoShow);
    expect(pref.agentRegex).toBe("^aider");
  });
});

describe("enterAction — tabela de teclas da spec", () => {
  test("default: Enter envia, Shift+Enter quebra linha, Ctrl/Cmd+Enter nada", () => {
    expect(enterAction({ shift: false, ctrlOrMeta: false }, false)).toBe(
      "submit",
    );
    expect(enterAction({ shift: true, ctrlOrMeta: false }, false)).toBe(
      "newline",
    );
    expect(enterAction({ shift: false, ctrlOrMeta: true }, false)).toBe("none");
  });

  test("submitWithCtrlEnter: Enter quebra linha, Ctrl/Cmd+Enter envia", () => {
    expect(enterAction({ shift: false, ctrlOrMeta: false }, true)).toBe(
      "newline",
    );
    expect(enterAction({ shift: true, ctrlOrMeta: false }, true)).toBe(
      "newline",
    );
    expect(enterAction({ shift: false, ctrlOrMeta: true }, true)).toBe(
      "submit",
    );
  });
});

describe("atQuery — token @ ativo sob o caret", () => {
  test("detecta @ no início e após espaço", () => {
    expect(atQuery("@ma", 3)).toEqual({ start: 0, query: "ma" });
    expect(atQuery("veja @src/li", 12)).toEqual({ start: 5, query: "src/li" });
  });

  test("@ colado em palavra (email) não é token", () => {
    expect(atQuery("mande para x@y", 14)).toBeNull();
  });

  test("espaço fecha o token", () => {
    expect(atQuery("@main.rs pronto", 15)).toBeNull();
  });

  test("sem @ não há token", () => {
    expect(atQuery("texto comum", 11)).toBeNull();
  });

  test("caret no meio do token considera só o prefixo", () => {
    expect(atQuery("@src/lib.rs", 4)).toEqual({ start: 0, query: "src" });
  });
});

describe("insertToken", () => {
  test("substitui o token ativo pelo caminho e posiciona o caret após o espaço", () => {
    const result = insertToken("veja @ma e diga", 8, { start: 5, query: "ma" },
      "main.rs");
    expect(result.text).toBe("veja @main.rs  e diga");
    expect(result.caret).toBe(14);
  });
});

describe("toRelPath", () => {
  test("caminho dentro do cwd vira relativo", () => {
    expect(toRelPath("/repo/src/lib.rs", "/repo")).toBe("src/lib.rs");
  });

  test("fora do cwd ou sem cwd fica absoluto", () => {
    expect(toRelPath("/outro/x.rs", "/repo")).toBe("/outro/x.rs");
    expect(toRelPath("/outro/x.rs", null)).toBe("/outro/x.rs");
  });
});

describe("shouldShowRichInput — camadas de confiança e conveniência", () => {
  test("sessão de agente conhecida sempre mostra", () => {
    expect(
      shouldShowRichInput(
        { type: "agent", runner: "claude_code" },
        false,
        DEFAULT_RICH_INPUT,
      ),
    ).toBe(true);
  });

  test("shell com agent_match mostra só se a pref permite", () => {
    const shell = { type: "shell" } as const;
    expect(shouldShowRichInput(shell, true, DEFAULT_RICH_INPUT)).toBe(true);
    expect(
      shouldShowRichInput(shell, true, {
        ...DEFAULT_RICH_INPUT,
        showOnMatch: false,
      }),
    ).toBe(false);
  });

  test("shell sem match nunca mostra", () => {
    expect(
      shouldShowRichInput({ type: "shell" }, false, DEFAULT_RICH_INPUT),
    ).toBe(false);
  });
});

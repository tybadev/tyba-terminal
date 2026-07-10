import { describe, expect, test } from "bun:test";

import type { SessionCommand } from "./ipc";
import {
  DEFAULT_RICH_INPUT,
  atQuery,
  enterAction,
  insertToken,
  parseRichInputPref,
  richInputVisibility,
  shouldShowRichInput,
} from "./richInput";

const cmd = (over: Partial<SessionCommand> = {}): SessionCommand => ({
  command: null,
  running: false,
  agent_match: false,
  ...over,
});

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

describe("richInputVisibility", () => {
  const agent = { type: "agent", runner: "claude_code" } as const;
  const shell = { type: "shell" } as const;

  test("aberto explicitamente ignora autoShow e dismissed", () => {
    expect(
      richInputVisibility({
        kind: shell,
        command: cmd(),
        pref: { ...DEFAULT_RICH_INPUT, autoShow: false },
        opened: true,
        dismissed: true,
      }),
    ).toBe(true);
  });

  test("dismissed esconde mesmo com agente elegível e ocioso", () => {
    expect(
      richInputVisibility({
        kind: agent,
        command: cmd(),
        pref: DEFAULT_RICH_INPUT,
        opened: false,
        dismissed: true,
      }),
    ).toBe(false);
  });

  test("agente rodando não abre sozinho; ocioso abre", () => {
    const base = {
      kind: agent,
      pref: DEFAULT_RICH_INPUT,
      opened: false,
      dismissed: false,
    };
    expect(
      richInputVisibility({ ...base, command: cmd({ running: true }) }),
    ).toBe(false);
    expect(
      richInputVisibility({ ...base, command: cmd({ running: false }) }),
    ).toBe(true);
  });

  test("shell só aparece com agent_match e pref ligada", () => {
    const base = { kind: shell, opened: false, dismissed: false };
    expect(
      richInputVisibility({
        ...base,
        command: cmd({ agent_match: true }),
        pref: DEFAULT_RICH_INPUT,
      }),
    ).toBe(true);
    expect(
      richInputVisibility({
        ...base,
        command: cmd({ agent_match: false }),
        pref: DEFAULT_RICH_INPUT,
      }),
    ).toBe(false);
    expect(
      richInputVisibility({
        ...base,
        command: cmd({ agent_match: true }),
        pref: { ...DEFAULT_RICH_INPUT, showOnMatch: false },
      }),
    ).toBe(false);
  });

  test("autoShow desligado só mostra com abertura explícita", () => {
    expect(
      richInputVisibility({
        kind: agent,
        command: cmd(),
        pref: { ...DEFAULT_RICH_INPUT, autoShow: false },
        opened: false,
        dismissed: false,
      }),
    ).toBe(false);
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

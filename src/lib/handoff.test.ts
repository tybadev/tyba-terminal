import { describe, expect, mock, test } from "bun:test";

// O invoke do Tauri vira um gravador: o teste vê exatamente o que sai do webview.
const calls: Array<{ cmd: string; args: unknown }> = [];
mock.module("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args: unknown) => {
    calls.push({ cmd, args });
    return Promise.resolve(null);
  },
}));

const { handoffTargets, sessionHandoffOpen, sessionHandoffPreview } = await import("./ipc");
const { defaultTarget, handoffReady, handoffSource, noteAsSent, previewLines } = await import(
  "./handoff"
);

const turn = {
  seq: 1,
  outcome: "completed",
  failure: null,
  started_at_ms: 0,
  ended_at_ms: 1,
  duration_ms: 1,
  prompt: "corrige o parser",
  diff: "ok",
  files: [],
  files_total: 0,
  insertions: 0,
  deletions: 0,
  commits: 0,
} as never;

describe("handoff: o que sai para o core", () => {
  test("preview e open levam só id, agente e nota — nunca o texto", async () => {
    calls.length = 0;
    await sessionHandoffPreview("s1", "codex", "tira o TODO");
    await sessionHandoffOpen("s1", "codex", "tira o TODO");
    await handoffTargets("s1");
    expect(calls).toEqual([
      { cmd: "session_handoff_preview", args: { id: "s1", target: "codex", note: "tira o TODO" } },
      { cmd: "session_handoff_open", args: { id: "s1", target: "codex", note: "tira o TODO" } },
      { cmd: "handoff_targets", args: { id: "s1" } },
    ]);
    for (const c of calls) expect(Object.keys(c.args as object)).not.toContain("text");
  });
});

describe("handoffReady", () => {
  test("turno fechado com resumo: liga", () => {
    expect(handoffReady({ last_turn: turn, turn_open: false })).toBe(true);
  });
  test("turno aberto: desliga, mesmo com o resumo do anterior", () => {
    expect(handoffReady({ last_turn: turn, turn_open: true })).toBe(false);
  });
  test("sem turno nenhum: desliga", () => {
    expect(handoffReady({ last_turn: null })).toBe(false);
  });
});

describe("defaultTarget", () => {
  test("pula o indisponível (Gemini sem runner)", () => {
    expect(
      defaultTarget([
        { agent: "gemini", label: "Gemini", available: false, reason: "sem suporte ainda" },
        { agent: "codex", label: "Codex", available: true, reason: null },
      ]),
    ).toBe("codex");
  });
  test("nenhum disponível: nada marcado", () => {
    expect(
      defaultTarget([
        { agent: "gemini", label: "Gemini", available: false, reason: "sem suporte ainda" },
      ]),
    ).toBeNull();
  });
});

// O `body` do core: o texto sem o bloco da nota, com a fala citada em "> ".
const BODY = [
  "Handoff de Claude · tyba-terminal",
  "",
  "Pedido:",
  "fecha o bug",
  "",
  "O que o Claude disse por último:",
  "> Rodei com GH_TOKEN=[REDACTED] do .env.test.",
  "> Resumo:",
  "",
  "O que mudou (1 arquivos, +12 −3):",
  "- src/osc133.rs +12 −3",
  "",
].join("\n");

describe("previewLines", () => {
  const lines = previewLines(BODY);
  test("desenha o body inteiro, sem a linha vazia do fim", () => {
    expect(lines).toHaveLength(11);
  });
  test("cabeçalho e títulos de seção em destaque", () => {
    expect(lines[0].head).toBe(true);
    expect(lines[2].head).toBe(true);
    expect(lines[3].head).toBe(false);
    expect(lines[5].head).toBe(true);
  });
  test("linha citada nunca vira título, mesmo terminando em ':'", () => {
    expect(lines[6]).toMatchObject({ head: false, quote: true });
    expect(lines[7]).toMatchObject({ head: false, quote: true });
    expect(previewLines("> Pedido:")[0]).toMatchObject({ head: false, quote: true });
  });
  test("a marca de redação vira peça própria", () => {
    expect(lines[6].parts.map((p) => p.kind)).toEqual(["text", "mark", "text"]);
  });
  test("+/− da linha de arquivo", () => {
    expect(lines[10].parts.slice(-3)).toEqual([
      { kind: "add", text: "+12" },
      { kind: "text", text: " " },
      { kind: "del", text: "−3" },
    ]);
  });
});

describe("noteAsSent", () => {
  test("igual ao digitado (trim): nada a mostrar", () => {
    expect(noteAsSent("tira o TODO", "  tira o TODO ", "  tira o TODO ")).toBeNull();
  });
  test("o core mudou a nota (redação): mostra como vai", () => {
    expect(noteAsSent("usa [REDACTED]", "usa sk-abc", "usa sk-abc")).toBe("usa [REDACTED]");
  });
  test("texto ainda de uma nota anterior: não compara", () => {
    expect(noteAsSent("usa [REDACTED]", "usa sk-abc", "usa sk-abc!")).toBeNull();
  });
  test("sem texto do core: nada", () => {
    expect(noteAsSent(undefined, null, "x")).toBeNull();
  });
});

describe("handoffSource", () => {
  test("claude e codex passam; runner custom e sem agente, não", () => {
    expect(handoffSource("claude_code")).toBe("claude");
    expect(handoffSource("codex")).toBe("codex");
    expect(handoffSource({ custom: "aider" })).toBeNull();
    expect(handoffSource(null)).toBeNull();
  });
});

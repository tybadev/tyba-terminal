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
const { defaultTarget, handoffReady, previewBody, previewLines } = await import("./handoff");

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

const TEXT = [
  "Handoff de Claude · tyba-terminal",
  "",
  "Pedido:",
  "fecha o bug",
  "",
  "O que o Claude disse por último:",
  "Rodei com GH_TOKEN=[REDACTED] do .env.test.",
  "",
  "O que mudou (1 arquivos, +12 −3):",
  "- src/osc133.rs +12 −3",
  "",
  "Nota do dono (o que fazer agora):",
  "tira o TODO",
  "",
].join("\n");

describe("previewBody", () => {
  test("com nota: o bloco da nota sai do texto (vira o campo)", () => {
    const body = previewBody(TEXT, "tira o TODO");
    expect(body).not.toContain("Nota do dono");
    expect(body.endsWith("- src/osc133.rs +12 −3")).toBe(true);
  });
  test("sem nota: o texto fica inteiro", () => {
    expect(previewBody("a\nb\n", "")).toBe("a\nb");
  });
});

describe("previewLines", () => {
  const lines = previewLines(previewBody(TEXT, "tira o TODO"));
  test("cabeçalho e títulos de seção em destaque", () => {
    expect(lines[0].head).toBe(true);
    expect(lines[2].head).toBe(true);
    expect(lines[3].head).toBe(false);
  });
  test("a marca de redação vira peça própria", () => {
    expect(lines[6].parts.map((p) => p.kind)).toEqual(["text", "mark", "text"]);
  });
  test("+/− da linha de arquivo", () => {
    expect(lines[9].parts.slice(-3)).toEqual([
      { kind: "add", text: "+12" },
      { kind: "text", text: " " },
      { kind: "del", text: "−3" },
    ]);
  });
});

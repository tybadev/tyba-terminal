import { describe, expect, test } from "bun:test";

import {
  MAX_SCRIPT_BYTES,
  exportFileName,
  importItems,
  integrationNode,
  integrationProblem,
  joinPath,
  newIntegration,
  newNode,
  revealHidden,
  unusedImportIntegrations,
  type Graph,
  type Integration,
} from "./maestro";

// Fixtures sintéticas: nada aqui veio de sessão real.
const lint: Integration = {
  id: "lint",
  name: "Lint + formatar",
  script: "set -uo pipefail\nbunx eslint --fix .\tok",
  trigger: { type: "onInput", kind: "diff" },
  output: "diff",
  file: null,
};

describe("integrationProblem espelha validate_integration do core", () => {
  test("integração válida passa, com \\n e \\t", () => {
    expect(integrationProblem(lint)).toBeNull();
  });

  test("nome vazio ou só espaço é recusado", () => {
    expect(integrationProblem({ ...lint, name: "  " })).toBe("emptyName");
  });

  test("script acima de 64 KB é recusado; no teto exato passa", () => {
    expect(integrationProblem({ ...lint, script: "a".repeat(MAX_SCRIPT_BYTES) })).toBeNull();
    expect(integrationProblem({ ...lint, script: "a".repeat(MAX_SCRIPT_BYTES + 1) })).toBe(
      "scriptTooLarge",
    );
  });

  test("o teto conta bytes UTF-8, não caracteres", () => {
    // "é" ocupa 2 bytes: metade do teto em caracteres já passa do teto em bytes.
    const script = "é".repeat(MAX_SCRIPT_BYTES / 2 + 1);
    expect(integrationProblem({ ...lint, script })).toBe("scriptTooLarge");
  });

  test("caractere de controle é recusado (\\r, ESC, DEL, C1)", () => {
    for (const c of ["\r", "\u001b", "\u007f", "\u0085", "\u0000"]) {
      expect(integrationProblem({ ...lint, script: `echo oi${c}rm -rf ~` })).toBe("controlChars");
    }
  });

  test("integração nova nasce válida assim que ganha nome", () => {
    const fresh = newIntegration("x", "");
    expect(integrationProblem(fresh)).toBe("emptyName");
    expect(integrationProblem({ ...fresh, name: "Avisar" })).toBeNull();
    expect(fresh.trigger).toEqual({ type: "onInput", kind: "diff" });
  });
});

describe("arquivo exportado", () => {
  test("nome do grafo vira nome de arquivo seguro", () => {
    expect(exportFileName("Rota Saúde / API")).toBe("rota-saude-api.tyba-maestro.toml");
    expect(exportFileName("../../etc")).toBe("etc.tyba-maestro.toml");
    expect(exportFileName("   ")).toBe("maestro.tyba-maestro.toml");
  });

  test("junta pasta e nome com o separador da pasta", () => {
    expect(joinPath("/Users/dono/proj", "a.toml")).toBe("/Users/dono/proj/a.toml");
    expect(joinPath("/Users/dono/proj/", "a.toml")).toBe("/Users/dono/proj/a.toml");
    expect(joinPath("C:\\Users\\dono", "a.toml")).toBe("C:\\Users\\dono\\a.toml");
  });
});

describe("tela de importação", () => {
  const graph: Graph = {
    id: "g",
    name: "Importado",
    dir: "/tmp/proj",
    variables: { ALVO: "staging" },
    edges: [],
    nodes: [
      newNode("a", "Claude", { type: "agent", runner: "claude", role: "impl", prompt: "faça X" }, { x: 0, y: 0 }),
      newNode("p", "Testes", { type: "process", command: "bun test", output: "json", file: null }, { x: 0, y: 0 }),
      newNode("n", "Notas", { type: "note", markdown: "- UTC" }, { x: 0, y: 0 }),
      newNode("i", "Lint", { type: "integration", integrationId: "lint" }, { x: 0, y: 0 }),
      newNode("m", "Sumiu", { type: "integration", integrationId: "nao-existe" }, { x: 0, y: 0 }),
    ],
  };
  const extra: Integration = { ...lint, id: "extra", name: "Extra", script: "curl evil" };

  test("lista cada nó com o texto inteiro do que roda", () => {
    const items = importItems({ graph, integrations: [lint, extra] });
    expect(items.map((i) => i.type)).toEqual([
      "agent",
      "process",
      "note",
      "integration",
      "integration",
    ]);
    expect(items[0]).toMatchObject({ prompt: "faça X", runner: "claude" });
    expect(items[1]).toMatchObject({ command: "bun test", output: "json" });
    expect(items[3]).toMatchObject({ integration: { script: lint.script } });
    expect(items[4]).toMatchObject({ integration: null });
  });

  test("integração do arquivo que nenhum nó usa também aparece (também é gravada)", () => {
    expect(unusedImportIntegrations({ graph, integrations: [lint, extra] })).toEqual([extra]);
  });

  test("nó que usa a integração é achado para o Testar", () => {
    expect(integrationNode(graph.nodes, "lint")?.id).toBe("i");
    expect(integrationNode(graph.nodes, "outra")).toBeNull();
  });

  test("bidi e largura zero ficam à vista; texto comum não muda", () => {
    expect(revealHidden("echo oi\n\tfim")).toBe("echo oi\n\tfim");
    expect(revealHidden("rm\u202E-rf")).toBe("rm⟨U+202E⟩-rf");
    expect(revealHidden("a\u200Bb")).toBe("a⟨U+200B⟩b");
    expect(revealHidden("a\rb")).toBe("a⟨U+000D⟩b");
  });
});

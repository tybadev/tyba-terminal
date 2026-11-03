import { describe, expect, test } from "bun:test";

import {
  MAX_ROLE_INSTRUCTIONS_CHARS,
  MAX_ROLE_NAME_CHARS,
  copyName,
  roleProblem,
  roleSummary,
  roleUsage,
  type Graph,
  type OrbitNode,
  type Role,
} from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
const revisor: Role = {
  name: "Revisor",
  instructions: "Você revisa o diff de outro agente.\nAponte risco e regressão.\tcite arquivo e linha",
};

describe("roleProblem espelha validate_role do core", () => {
  test("papel válido passa, com \\n e \\t nas instruções", () => {
    expect(roleProblem(revisor)).toBeNull();
  });

  test("nome vazio, com quebra de linha ou acima de 40 é recusado", () => {
    expect(roleProblem({ ...revisor, name: "  " })).toBe("emptyName");
    expect(roleProblem({ ...revisor, name: "a\nb" })).toBe("nameOneLine");
    expect(roleProblem({ ...revisor, name: "a b" })).toBe("nameOneLine");
    expect(roleProblem({ ...revisor, name: "x".repeat(MAX_ROLE_NAME_CHARS) })).toBeNull();
    expect(roleProblem({ ...revisor, name: "x".repeat(MAX_ROLE_NAME_CHARS + 1) })).toBe(
      "nameTooLong",
    );
  });

  test("instruções acima de 4000 caracteres ou com controle são recusadas", () => {
    expect(
      roleProblem({ ...revisor, instructions: "é".repeat(MAX_ROLE_INSTRUCTIONS_CHARS) }),
    ).toBeNull();
    expect(
      roleProblem({ ...revisor, instructions: "é".repeat(MAX_ROLE_INSTRUCTIONS_CHARS + 1) }),
    ).toBe("instructionsTooLong");
    expect(roleProblem({ ...revisor, instructions: "a\u001b[2Jb" })).toBe("controlChars");
  });
});

describe("uso do papel e nomes", () => {
  const agent = (id: string, role: string): OrbitNode => ({
    id,
    title: id,
    kind: { type: "agent", runner: "claude", role, prompt: "x", dir: "/tmp/x" },
    position: { x: 0, y: 0 },
    size: { w: 320, h: 200 },
  });
  const graph = (id: string, roles: string[]): Pick<Graph, "nodes"> => ({
    nodes: roles.map((r, i) => agent(`${id}-${i}`, r)),
  });

  test("roleUsage conta nós e grafos de toda a máquina", () => {
    const graphs = [graph("a", ["Revisor", "Revisor", "QA"]), graph("b", ["Revisor"]), graph("c", [])];
    expect(roleUsage(graphs, "Revisor")).toEqual({ nodes: 3, graphs: 2 });
    expect(roleUsage(graphs, "QA")).toEqual({ nodes: 1, graphs: 1 });
    expect(roleUsage(graphs, "Documentador")).toEqual({ nodes: 0, graphs: 0 });
  });

  test("roleSummary é a primeira linha com texto", () => {
    expect(roleSummary("\n\n  lê o diff  \nsegunda")).toBe("lê o diff");
    expect(roleSummary("")).toBe("");
  });

  test("copyName não repete e fica no teto", () => {
    expect(copyName("QA", ["QA"])).toBe("QA (cópia)");
    expect(copyName("QA", ["QA", "QA (cópia)"])).toBe("QA (cópia 2)");
    const long = copyName("x".repeat(MAX_ROLE_NAME_CHARS), []);
    expect([...long].length).toBeLessThanOrEqual(MAX_ROLE_NAME_CHARS);
    expect(long.endsWith(" (cópia)")).toBe(true);
  });
});

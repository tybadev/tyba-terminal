import { describe, expect, test } from "bun:test";

import { filterPalette, paletteEntries, recentDirs, type PaletteInput } from "./graphPalette";
import type { Integration, NodeKind } from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
const labels = {
  claude: "Agente Claude",
  codex: "Agente Codex",
  process: "Processo",
  service: "Serviço",
  browser: "Navegador",
  note: "Nota",
};

const integration: Integration = {
  id: "pr",
  name: "Abrir PR",
  script: "gh pr create",
  trigger: { type: "onInput", kind: "diff" },
  output: "log",
  file: null,
};

const input: PaletteInput = {
  labels,
  dir: "/srv/api",
  integrations: [integration],
  snippets: [{ id: "s1", name: "subir front", command: "bun dev" }],
  dirs: ["/srv/web", "/srv/web", ""],
};

const entries = paletteEntries(input);
const labelsOf = (list: { label: string }[]) => list.map((e) => e.label);

describe("paletteEntries", () => {
  test("tipos na ordem do quadro, depois integrações, snippets e pastas", () => {
    expect(entries.map((e) => e.group)).toEqual([
      "type",
      "type",
      "type",
      "type",
      "type",
      "type",
      "integration",
      "snippet",
      "folder",
    ]);
    expect(labelsOf(entries.slice(0, 6))).toEqual(Object.values(labels));
  });

  test("snippet vira Processo com o comando, na pasta do nó novo", () => {
    const s = entries.find((e) => e.group === "snippet")!;
    expect(s.kind).toEqual({
      type: "process",
      command: "bun dev",
      output: "log",
      file: null,
      dir: "/srv/api",
    });
    expect(s.title).toBe("subir front");
  });

  test("pasta recente abre um Claude nela (sem repetir, sem vazia)", () => {
    const folders = entries.filter((e) => e.group === "folder");
    expect(folders).toHaveLength(1);
    expect(folders[0].kind).toMatchObject({ type: "agent", runner: "claude", dir: "/srv/web" });
  });
});

describe("filterPalette", () => {
  test("sem busca: só os tipos", () => {
    expect(labelsOf(filterPalette(entries, "  "))).toEqual(Object.values(labels));
  });

  test("busca em todos os grupos, sem acento e sem caixa", () => {
    expect(labelsOf(filterPalette(entries, "servico"))).toEqual(["Serviço"]);
    expect(labelsOf(filterPalette(entries, "abrir"))).toEqual(["Abrir PR"]);
    expect(labelsOf(filterPalette(entries, "bun"))).toEqual(["subir front"]);
  });

  test("começa com a busca vem antes de só contém", () => {
    // "Processo" começa com "pr"; "Abrir PR" só tem uma palavra que começa.
    expect(labelsOf(filterPalette(entries, "pr"))).toEqual(["Processo", "Abrir PR"]);
    // A pasta recente acha pelo que ela abre (o Claude), depois dos tipos.
    expect(labelsOf(filterPalette(entries, "agente"))).toEqual([
      "Agente Claude",
      "Agente Codex",
      "/srv/web",
    ]);
    expect(labelsOf(filterPalette(entries, "claude"))[0]).toBe("Agente Claude");
  });

  test("ligação de url: Navegador primeiro e os tipos que aceitam embaixo (quadro)", () => {
    const source: NodeKind = {
      type: "process",
      command: "bun dev",
      output: "log",
      file: null,
      dir: "",
    };
    const link = { source, integrations: new Map([[integration.id, integration]]) };
    const nav = filterPalette(entries, "nav", link);
    expect(labelsOf(nav)).toEqual(["Navegador", "Agente Claude", "Agente Codex", "Processo"]);
    // Nota e Serviço não aceitam nada; a integração só aceita diff.
    expect(labelsOf(filterPalette(entries, "", link))).toEqual([
      "Agente Claude",
      "Agente Codex",
      "Processo",
      "Navegador",
    ]);
  });

  test("ligação de agente (diff): a integração de diff entra ao buscar", () => {
    const source: NodeKind = { type: "agent", runner: "claude", role: "", prompt: "", dir: "" };
    const link = { source, integrations: new Map([[integration.id, integration]]) };
    expect(labelsOf(filterPalette(entries, "abrir", link))[0]).toBe("Abrir PR");
    // O agente também liga no navegador (controla) e na nota (plano).
    expect(labelsOf(filterPalette(entries, "", link))).toContain("Navegador");
  });
});

describe("recentDirs", () => {
  test("a última usada primeiro, depois as dos nós, sem repetir", () => {
    const kinds: NodeKind[] = [
      { type: "note", markdown: "" },
      { type: "process", command: "", output: "log", file: null, dir: "/a" },
      { type: "agent", runner: "codex", role: "", prompt: "", dir: "/b" },
      { type: "integration", integrationId: "x", dir: "/a" },
    ];
    expect(recentDirs("/b", kinds)).toEqual(["/b", "/a"]);
    expect(recentDirs(null, [])).toEqual([]);
  });
});

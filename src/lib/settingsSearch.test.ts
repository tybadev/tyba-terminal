import { describe, expect, it } from "bun:test";
import { searchSettings } from "./settingsSearch";

const entries = [
  { section: "general", label: "Idioma" },
  { section: "notifications", label: "Pedido de aprovação", hint: "Som quando um agente pede permissão" },
  { section: "terminal", label: "Gravar histórico", hint: "Fica no seu Mac" },
  { section: "appearance", label: "Notificações na sidebar" },
];

describe("searchSettings", () => {
  it("ignora acento e caixa", () => {
    expect(searchSettings(entries, "IDIOMA").map((e) => e.label)).toEqual(["Idioma"]);
    expect(searchSettings(entries, "historico").map((e) => e.label)).toEqual(["Gravar histórico"]);
  });

  it("nome casa antes de descrição", () => {
    expect(searchSettings(entries, "notificacoes").map((e) => e.label)).toEqual([
      "Notificações na sidebar",
    ]);
    expect(searchSettings(entries, "som").map((e) => e.label)).toEqual(["Pedido de aprovação"]);
  });

  it("todas as palavras precisam aparecer; vazio não acha nada", () => {
    expect(searchSettings(entries, "gravar mac")).toHaveLength(1);
    expect(searchSettings(entries, "gravar docker")).toHaveLength(0);
    expect(searchSettings(entries, "  ")).toHaveLength(0);
  });
});

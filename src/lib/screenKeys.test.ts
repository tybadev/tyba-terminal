import { describe, expect, test } from "bun:test";

const APP = await Bun.file(new URL("../App.tsx", import.meta.url)).text();

/** Git, Docker e Serviços são irmãos no `<main>` e podem ser da MESMA sessão
 * (um saindo, animado, enquanto o outro entra). Com o id puro como key, o React
 * via duas crianças com a mesma key e deixava a tela órfã no DOM — viva e sem
 * fechar (abrir Docker → abrir Serviços → fechar → o Docker não fecha mais,
 * 2026-11-02). A key leva o tipo da tela. */
describe("telas cheias com key própria", () => {
  test("nenhuma tela cheia usa só o id da sessão como key", () => {
    expect(APP).not.toMatch(/key=\{(git|docker|services)ScreenSession\.id\}/);
  });

  test("cada uma leva o tipo na key", () => {
    expect(APP).toContain("key={`git:${gitScreenSession.id}`}");
    expect(APP).toContain("key={`docker:${dockerScreenSession.id}`}");
    expect(APP).toContain("key={`services:${servicesScreenSession.id}`}");
  });
});

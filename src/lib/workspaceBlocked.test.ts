import { describe, expect, test } from "bun:test";

const APP = await Bun.file(new URL("../App.tsx", import.meta.url)).text();

/** O Workspace está bloqueado: ele não é a view de worktrees — é pra algo maior, e
 * volta quando estiver pronto. O bloqueio é fácil de desfazer sem querer num
 * refactor (basta alguém religar um botão), então ele é verificado, não confiado. */
describe("workspace bloqueado", () => {
  test("nenhum caminho da UI abre a view de workspace", () => {
    const opens = APP.match(/openViewTab\(\s*["']workspace["']/g) ?? [];
    expect(opens).toEqual([]);
  });

  test("os acessos que existiam continuam visíveis, porém desabilitados", () => {
    // Sumir com o botão esconde a intenção; desabilitado com "em breve" conta o
    // que está acontecendo. Some com o rótulo e o bloqueio vira botão quebrado.
    expect(APP).toContain("comingSoon");
    const disabled = APP.match(/disabled\s*\n\s*aria-disabled|aria-disabled\s*\n\s*disabled/g) ?? [];
    expect(disabled.length).toBeGreaterThan(0);
  });
});

import { describe, expect, test } from "bun:test";

const APP = await Bun.file(new URL("../App.tsx", import.meta.url)).text();

/** O Workspace não é a view antiga de worktrees: ela segue fechada, e isso é fácil
 * de desfazer sem querer num refactor, então é verificado, não confiado. O item
 * Workspace da lateral agora abre o Orbit (2026-11-02). */
describe("workspace: a view antiga segue fechada, o Orbit abre pela lateral", () => {
  test("nenhum caminho da UI abre a view de workspace", () => {
    const opens = APP.match(/openViewTab\(\s*["']workspace["']/g) ?? [];
    expect(opens).toEqual([]);
  });

  test("o item Orbit da lateral abre e fecha o Orbit", () => {
    // Decisão do dono (2026-11-02): o Workspace deixou de ser "em breve" — é
    // por ele, na lateral, que se entra no Orbit (e não por um seletor na
    // barra). Ele alterna o Orbit e acende enquanto o Orbit está aberto.
    expect(APP).toMatch(/aria-label=\{t\("orbitView"\)\}\s*\n\s*aria-pressed=\{orbitOpen\}/);
    expect(APP).toContain("setOrbitOpen((o) => !o)");
  });
});

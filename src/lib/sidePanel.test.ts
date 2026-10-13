import { describe, expect, test } from "bun:test";

import { mainColumn } from "./agentsPanel";
import {
  escClosesPanel,
  isNarrowPanel,
  litPanel,
  panelAriaLabel,
  panelClick,
  panelIndicator,
  parseSideView,
  visiblePanels,
} from "./sidePanel";

const t = (key: string, options?: Record<string, unknown>) =>
  options && "count" in options ? `${key}:${options.count}` : key;

const base = {
  dockerUp: true,
  dockerRunning: 0,
  gitDirty: false,
  pendingApprovals: 0,
};

describe("parseSideView", () => {
  test("lê cada prefixo do core", () => {
    expect(parseSideView("git:s1")).toEqual({ kind: "git", session: "s1" });
    expect(parseSideView("docker:s1")).toEqual({ kind: "docker", session: "s1" });
    expect(parseSideView("diff:s1")).toEqual({ kind: "diff", session: "s1" });
    expect(parseSideView("files:s1")).toEqual({ kind: "files", session: "s1" });
    expect(parseSideView("agents:s1")).toEqual({ kind: "agents", session: "s1" });
    expect(parseSideView("tunnels:s1")).toEqual({ kind: "tunnels", session: "s1" });
    expect(parseSideView("history:s1")).toEqual({ kind: "history", session: "s1" });
  });

  test("fila de agentes, vazio e lixo não são aba de sessão", () => {
    expect(parseSideView("agent-queue")).toBeNull();
    expect(parseSideView(null)).toBeNull();
    expect(parseSideView("git:")).toBeNull();
    expect(parseSideView("stats")).toBeNull();
  });
});

describe("litPanel", () => {
  test("acende o painel aberto da sessão em foco", () => {
    expect(litPanel("git:s1", "s1")).toBe("git");
    expect(litPanel("diff:s1", "s1")).toBe("diff");
  });

  test("painel de outra sessão do workspace não acende", () => {
    expect(litPanel("git:s2", "s1")).toBeNull();
  });

  test("nada aberto, nada aceso", () => {
    expect(litPanel(null, "s1")).toBeNull();
    expect(litPanel("agent-queue", "s1")).toBeNull();
  });
});

describe("panelClick", () => {
  test("ícone aceso fecha; apagado abre (um por vez)", () => {
    expect(panelClick("git", "git:s1", "s1")).toBe("close");
    expect(panelClick("docker", "git:s1", "s1")).toBe("open");
    expect(panelClick("git", null, "s1")).toBe("open");
  });

  test("painel aberto para outra sessão: clicar abre o desta", () => {
    expect(panelClick("git", "git:s2", "s1")).toBe("open");
  });

  test("Agentes com a fila aberta troca pela aba da sessão", () => {
    expect(panelClick("agents", "agent-queue", "s1")).toBe("open");
  });

  test("histórico é aba como as outras", () => {
    expect(panelClick("history", "git:s1", "s1")).toBe("open");
    expect(panelClick("history", "history:s1", "s1")).toBe("close");
  });
});

describe("isNarrowPanel", () => {
  test("todas as abas e a fila usam os 380px fixos", () => {
    for (const v of [
      "git:s",
      "docker:s",
      "agents:s",
      "tunnels:s",
      "files:s",
      "history:s",
      "agent-queue",
    ]) {
      expect(isNarrowPanel(v)).toBe(true);
    }
  });

  test("só o diff segue no split arrastável", () => {
    expect(isNarrowPanel("diff:s")).toBe(false);
    expect(isNarrowPanel(null)).toBe(false);
  });

  test("a coluna principal reflete a largura fixa do git", () => {
    expect(mainColumn("git:s1", false, null, false, 0.5).slot).toBe("narrow");
    expect(mainColumn("git:s1", false, null, true, 0.5).slot).toBe("hidden");
  });
});

describe("panelIndicator", () => {
  test("docker: verde rodando, vermelho indisponível, nada parado", () => {
    expect(panelIndicator("docker", { ...base, dockerRunning: 3 }).dot).toBe("green");
    expect(panelIndicator("docker", { ...base, dockerUp: false }).dot).toBe("red");
    expect(panelIndicator("docker", base)).toEqual({ dot: null, count: null });
  });

  test("git: âmbar com alteração", () => {
    expect(panelIndicator("git", { ...base, gitDirty: true }).dot).toBe("amber");
    expect(panelIndicator("git", base).dot).toBeNull();
  });

  test("agentes: selo com a contagem de aprovações", () => {
    expect(panelIndicator("agents", { ...base, pendingApprovals: 2 })).toEqual({
      dot: null,
      count: 2,
    });
    expect(panelIndicator("agents", base).count).toBeNull();
  });

  test("demais abas não mostram indicador", () => {
    for (const kind of ["history", "files", "diff", "tunnels"] as const) {
      expect(panelIndicator(kind, { ...base, gitDirty: true, dockerRunning: 2 })).toEqual({
        dot: null,
        count: null,
      });
    }
  });
});

describe("panelAriaLabel", () => {
  test("leva as contagens que o ícone mostra", () => {
    expect(panelAriaLabel("docker", { ...base, dockerRunning: 3, gitChanged: null }, t)).toBe(
      "sidePanelDockerRunning:3",
    );
    expect(
      panelAriaLabel("git", { ...base, gitDirty: true, gitChanged: 4 }, t),
    ).toBe("sidePanelGitChanges:4");
    expect(
      panelAriaLabel("agents", { ...base, pendingApprovals: 1, gitChanged: null }, t),
    ).toBe("sidePanelAgentsPending:1");
  });

  test("sem estado, só o nome", () => {
    const input = { ...base, gitChanged: null };
    expect(panelAriaLabel("git", input, t)).toBe("sidePanelGit");
    expect(panelAriaLabel("agents", input, t)).toBe("agentsAction");
    expect(panelAriaLabel("docker", { ...input, dockerUp: false }, t)).toBe(
      "sidePanelDockerDown",
    );
  });
});

describe("visiblePanels", () => {
  const all = {
    hasSession: true,
    isSsh: true,
    showDocker: true,
    hasRepo: true,
    agentsAvailable: true,
  };

  test("ordem do mockup", () => {
    expect(visiblePanels(all)).toEqual([
      "history",
      "docker",
      "tunnels",
      "files",
      "git",
      "agents",
    ]);
  });

  test("túneis só em SSH; git só com repo (Diff é seção da tela de Git); docker pela preferência", () => {
    expect(
      visiblePanels({ ...all, isSsh: false, hasRepo: false, showDocker: false, agentsAvailable: false }),
    ).toEqual(["history", "files"]);
  });

  test("sem sessão, sem abas", () => {
    expect(visiblePanels({ ...all, hasSession: false })).toEqual([]);
  });
});

describe("escClosesPanel", () => {
  test("Esc fecha, a não ser num campo ou já tratado", () => {
    const ev = { key: "Escape", defaultPrevented: false, targetEditable: false };
    expect(escClosesPanel(ev)).toBe(true);
    expect(escClosesPanel({ ...ev, targetEditable: true })).toBe(false);
    expect(escClosesPanel({ ...ev, defaultPrevented: true })).toBe(false);
    expect(escClosesPanel({ ...ev, key: "Enter" })).toBe(false);
  });
});

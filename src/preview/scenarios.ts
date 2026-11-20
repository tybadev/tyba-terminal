// Cada `?screen=` abre a tela do jeito que o dono abriria: pela intenção do
// próprio app (menu, ação de store, clique), nunca montando componente à parte.

import { emit } from "@tauri-apps/api/event";
import { EditorView } from "@codemirror/view";

import { openGitScreen } from "../lib/gitScreen";
import { requestHandoff } from "../lib/handoff";
import { dockerScreenSelect, openDockerPanel, openFilesPanel } from "../lib/ipc";
import type { setup } from "./core";
import { ID, REPO, TURN_HANDOFF } from "./fixtures";
import { GIT_SESSION } from "./fixtures-screens";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function waitFor<T>(find: () => T | null | undefined, ms = 4000): Promise<T> {
  for (let t = 0; t < ms; t += 50) {
    const found = find();
    if (found) return found;
    await sleep(50);
  }
  throw new Error("[preview] não achei o alvo do cenário");
}

const byText = (selector: string, text: RegExp) =>
  [...document.querySelectorAll<HTMLElement>(selector)].find((el) =>
    text.test(el.textContent ?? ""),
  );

const menu = (action: string) => emit("tyba:menu", action);

/** Estado inicial do core falso por cenário (antes do app montar). */
export const SCENARIO_SETUP: Record<string, Parameters<typeof setup>[0]> = {
  "shell-split": { workspace: "ws-api", tab: "tab-api-split" },
  agents: {
    workspace: "ws-api",
    tab: "tab-api-claude",
    drawer: { session_id: ID.claudeApi, open: true, tab: "agents", outcome_seen: false },
  },
  handoff: { workspace: "ws-api", tab: "tab-api-claude" },
  "git-prs": { workspace: "ws-api", tab: "tab-api-claude", sessions: GIT_SESSION },
  "git-pr-detail": { workspace: "ws-api", tab: "tab-api-claude", sessions: GIT_SESSION },
  "git-ci": { workspace: "ws-api", tab: "tab-api-claude", sessions: GIT_SESSION },
  docker: { workspace: "ws-api", tab: "tab-api-claude" },
  files: {
    workspace: "ws-tyba-terminal",
    files: {
      section: "files",
      // Quadro: CLAUDE.md é a aba de prévia (itálico, sem X).
      tabs: ["src/components/TurnPanel.tsx", "src/components/HandoffSheet.tsx", "CLAUDE.md"].map((rel) => ({
        view: `editor:${ID.tyba}:${rel}`,
        preview: rel === "CLAUDE.md",
      })),
      active: `editor:${ID.tyba}:src/components/TurnPanel.tsx`,
      recent: [],
    },
    snapshots: [
      {
        ahead: 0,
        behind: 0,
        upstream: "origin/feat/handoff-agentes-ui",
        root: REPO.tyba,
        branch: "feat/handoff-agentes-ui",
        status: { dirty: true, changed: 4, insertions: 42, deletions: 7 },
      },
    ],
  },
};

export const SCENARIOS: Record<string, () => Promise<unknown> | void> = {
  main: () => {},
  sidebar: () => {},
  header: () => {},
  "shell-split": () => {},
  agents: () => {},
  "git-prs": () => openGitScreen(ID.claudeApi, "prs"),
  "git-pr-detail": () => openGitScreen(ID.claudeApi, "prs", 437),
  "git-ci": () => openGitScreen(ID.claudeApi, "ci"),
  files: async () => {
    await openFilesPanel(ID.tyba);
    // Como o dono faria: abre src e components na árvore...
    const row = (name: string) =>
      waitFor(() =>
        [...document.querySelectorAll<HTMLElement>("div.group")].find(
          (el) => el.textContent?.trim() === name,
        ),
      );
    (await row("src")).click();
    (await row("components")).click();
    // ...apaga o espaço no fim da linha 47 (aba suja), põe o cursor na
    // coluna 18 e rola até a 40 no topo, como no quadro.
    const content = await waitFor(() => document.querySelector<HTMLElement>(".cm-content"));
    const view = await waitFor(() => EditorView.findFromDOM(content));
    await sleep(200);
    const l47 = view.state.doc.line(47);
    view.dispatch({ changes: { from: l47.to - 1, to: l47.to } });
    const l40 = view.state.doc.line(40);
    view.dispatch({
      selection: { anchor: view.state.doc.line(47).from + 17 },
      effects: EditorView.scrollIntoView(l40.from, { y: "start", yMargin: 9 }),
    });
  },
  docker: async () => {
    await openDockerPanel(ID.claudeApi);
    await dockerScreenSelect("ws-api", "c-postgres18");
  },
  settings: () => menu("settings"),
  orbit: () => menu("toggleOrbit"),
  handoff: () => requestHandoff({ sessionId: ID.claudeApi, from: "claude", turn: TURN_HANDOFF }),
  inbox: async () => {
    const button = await waitFor(() =>
      document.querySelector<HTMLElement>('button[aria-label^="Notificações"], button[aria-label="Inbox"]'),
    );
    button.click();
  },
  "session-menu": async () => {
    const row = await waitFor(() => byText("button", /^core.*~\/projetos\/core/));
    const rect = row.getBoundingClientRect();
    row.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        clientX: rect.left + 40,
        clientY: rect.top + 20,
        button: 2,
      }),
    );
    const sub = await waitFor(() => byText('[role="menuitem"]', /^Cor e grupo/));
    sub.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse" }));
    sub.click();
  },
};

export async function runScenario(name: string): Promise<void> {
  const run = SCENARIOS[name];
  if (!run) {
    console.warn(`[preview] tela desconhecida: ${name}. Opções: ${Object.keys(SCENARIOS).join(", ")}`);
    return;
  }
  // O app pronto: a barra lateral já listou as sessões.
  await waitFor(() => byText("button", /^api/), 8000);
  await sleep(150);
  await run();
}

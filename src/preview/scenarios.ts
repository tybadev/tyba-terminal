// Cada `?screen=` abre a tela do jeito que o dono abriria: pela intenção do
// próprio app (menu, ação de store, clique), nunca montando componente à parte.

import { emit } from "@tauri-apps/api/event";

import { openGitScreen } from "../lib/gitScreen";
import { requestHandoff } from "../lib/handoff";
import { dockerScreenSelect, openDockerPanel, openFilesPanel } from "../lib/ipc";
import type { setup } from "./core";
import { ID, TURN_HANDOFF } from "./fixtures";

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
  "git-prs": { workspace: "ws-api", tab: "tab-api-claude" },
  "git-pr-detail": { workspace: "ws-api", tab: "tab-api-claude" },
  "git-ci": { workspace: "ws-api", tab: "tab-api-claude" },
  docker: { workspace: "ws-api", tab: "tab-api-claude" },
  files: {
    workspace: "ws-tyba-terminal",
    files: {
      section: "files",
      tabs: ["src/components/TurnPanel.tsx", "src/components/HandoffSheet.tsx", "CLAUDE.md"].map((rel) => ({
        view: `editor:${ID.tyba}:${rel}`,
        preview: false,
      })),
      active: `editor:${ID.tyba}:src/components/TurnPanel.tsx`,
      recent: [],
    },
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
  files: () => openFilesPanel(ID.tyba),
  docker: async () => {
    await openDockerPanel(ID.claudeApi);
    await dockerScreenSelect("ws-api", "c-postgres18");
  },
  settings: () => menu("settings"),
  "settings-appearance": async () => {
    await menu("settings");
    const item = await waitFor(() => byText("button", /^Aparência$/));
    item.click();
  },
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

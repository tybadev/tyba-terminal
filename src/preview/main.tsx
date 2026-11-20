// Entrada do preview (`preview.html`, só no `vite dev`): o app de verdade com
// o IPC do Tauri trocado por um core falso e dados sintéticos, para fotografar
// cada tela e comparar com os quadros. `?screen=<nome>&theme=light|dark`.

import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import { handle, setup, state } from "./core";
import { SCENARIO_SETUP, runScenario } from "./scenarios";

const params = new URLSearchParams(location.search);
const screen = params.get("screen") ?? "main";
setup(SCENARIO_SETUP[screen] ?? {});
state.theme = params.get("theme") === "light" ? "light" : "dark";
{
  const f = params.get("font");
  state.font = f === "grotesk" || f === "mono" ? f : null;
}
// O tema lê o modo salvo antes de perguntar ao core: começa limpo.
localStorage.clear();
localStorage.setItem("tyba.theme", state.theme);
localStorage.setItem("tyba.theme.migrated", "1");
if (state.font) localStorage.setItem("tyba.font", state.font);

mockWindows("main");
mockIPC((cmd, args) => handle(cmd, args as Record<string, unknown>), {
  shouldMockEvents: true,
});

await import("../main");
void runScenario(screen).catch((e) => console.error(e));

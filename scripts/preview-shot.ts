// Foto de uma tela do preview (`preview.html`) no Chrome headless, para
// comparar pixel a pixel com o quadro.
//
//   bun scripts/preview-shot.ts <screen> <out.png> [W] [H] [--theme light|dark] [--log]
//
// Usa o `vite dev` que já estiver na porta 1425; se não houver, sobe um e o
// derruba no fim. `--log` imprime o console da página (comandos sem fixture).

import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PORT = 1425;
const BASE = `http://localhost:${PORT}`;

const argv = process.argv.slice(2);
const flag = (name: string) => {
  const i = argv.indexOf(name);
  if (i < 0) return null;
  const [, value] = argv.splice(i, 2);
  return value ?? null;
};
const theme = flag("--theme") ?? "light";
const logIdx = argv.indexOf("--log");
const log = logIdx >= 0;
if (log) argv.splice(logIdx, 1);
const [screen, out, w = "1110", h = "700"] = argv;
if (!screen || !out) {
  console.error("uso: bun scripts/preview-shot.ts <screen> <out.png> [W] [H] [--theme light|dark] [--log]");
  process.exit(2);
}

async function up(): Promise<boolean> {
  try {
    return (await fetch(`${BASE}/preview.html`)).ok;
  } catch {
    return false;
  }
}

let vite: ReturnType<typeof spawn> | null = null;
if (!(await up())) {
  vite = spawn("bunx", ["vite", "--port", String(PORT), "--strictPort"], {
    cwd: resolve(import.meta.dir, ".."),
    stdio: "ignore",
    // Grupo próprio: o `bunx` deixa o `vite` filho, e matar só o pai deixava
    // um servidor velho na porta servindo o código de antes.
    detached: true,
  });
  for (let i = 0; i < 100 && !(await up()); i++) await Bun.sleep(200);
  // Primeira carga otimiza dependências e recarrega a página: aquece antes.
  await fetch(`${BASE}/src/preview/main.tsx`).catch(() => {});
}

const profile = mkdtempSync(join(tmpdir(), "tyba-preview-"));
const url = `${BASE}/preview.html?screen=${encodeURIComponent(screen)}&theme=${theme}`;
const chrome = Bun.spawn(
  [
    CHROME,
    "--headless=new",
    `--user-data-dir=${profile}`,
    `--screenshot=${resolve(out)}`,
    `--window-size=${w},${h}`,
    "--hide-scrollbars",
    "--force-device-scale-factor=2",
    "--virtual-time-budget=8000",
    ...(log ? ["--enable-logging=stderr", "--v=0"] : []),
    url,
  ],
  { stdout: "ignore", stderr: "pipe" },
);
const stderr = await new Response(chrome.stderr).text();
const code = await chrome.exited;
rmSync(profile, { recursive: true, force: true });
if (vite?.pid) process.kill(-vite.pid);

if (log) {
  for (const line of stderr.split("\n")) {
    const m = line.match(/CONSOLE\(\d+\)\] "(.*)", source/) ?? line.match(/CONSOLE:\d+\] "(.*)", source/);
    if (m && !m[1].startsWith("[vite]") && !m[1].includes("React DevTools")) console.log(m[1]);
  }
}
if (code !== 0) {
  console.error(stderr);
  process.exit(code);
}
console.log(resolve(out));

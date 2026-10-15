// `bun run tauri <args>` passa por aqui. Em `dev`, injeta o
// src-tauri/tauri.dev.conf.json: o build de desenvolvimento vira outro app
// ("TYBA Dev", identificador dev.tyba.app.dev, ~/.tyba-dev) e não toca no banco
// nem na config do TYBA instalado. O core confere o identificador no arranque
// e recusa subir um build de dev com o de release. `build` e o resto passam
// intactos.
import { withDevConfig } from "./tauriArgs";

const args = withDevConfig(process.argv.slice(2));
const child = Bun.spawn(["tauri", ...args], {
  stdio: ["inherit", "inherit", "inherit"],
});
process.exit(await child.exited);

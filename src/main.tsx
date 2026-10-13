import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import "@fontsource/space-grotesk/500.css";
import "@fontsource/space-grotesk/600.css";
import "@fontsource/space-grotesk/700.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "./fonts.css";
import "./styles.css";
import "./i18n";
import "./theme";
import "./font";
import App from "./App";
import { SPLASH_DONE_EVENT } from "./lib/startup";

// O webview traz um menu de contexto próprio (Reload, e Inspect em debug) que
// não é do produto e aparece em toda área sem menu nosso. Campo de texto
// mantém o nativo — no CodeMirror e nos inputs ele ainda serve. Os menus do
// Radix continuam funcionando: eles já chamam preventDefault antes daqui.
document.addEventListener("contextmenu", (event) => {
  const target = event.target;
  if (
    target instanceof Element &&
    target.closest("input, textarea, [contenteditable='true']")
  ) {
    return;
  }
  event.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// O splash mede "o app está pronto", e não "o React montou".
//
// Ele saía dois `requestAnimationFrame` depois do `render()` — uns 32ms, com
// a lista de sessões ainda vazia e o layout ainda por vir. O usuário via
// splash → interface vazia → congelamento enquanto o resto chegava, e é essa
// sequência que faz "carregando" ser lido como "travou".
//
// Agora quem manda sair é o `App`, quando o core avisa que terminou de
// carregar. O splash é uma janela própria (`splash.html`) e esta, a `main`,
// nasce escondida: o aviso pede ao core para trocar uma pela outra. O teto
// (core parado num diálogo de permissão do macOS, webview que nem carregou)
// mora no core, em `splash::CEILING` — daqui não daria para cobrir o segundo.
window.addEventListener(
  SPLASH_DONE_EVENT,
  () => void invoke("splash_done").catch(() => {}),
  { once: true },
);

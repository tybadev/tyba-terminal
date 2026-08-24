import React from "react";
import ReactDOM from "react-dom/client";
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
import { SPLASH_CEILING_MS, SPLASH_DONE_EVENT } from "./lib/startup";

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
// carregar. Com teto: o core pode estar parado num diálogo de permissão do
// macOS, e splash eterno é pior que UI vazia.
const splash = document.getElementById("splash");
if (splash) {
  let done = false;
  const hide = () => {
    if (done) return;
    done = true;
    window.removeEventListener(SPLASH_DONE_EVENT, hide);
    splash.dataset.hidden = "true";
    const remove = () => splash.remove();
    splash.addEventListener("transitionend", remove, { once: true });
    window.setTimeout(remove, 600);
  };
  window.addEventListener(SPLASH_DONE_EVENT, hide);
  window.setTimeout(hide, SPLASH_CEILING_MS);
}

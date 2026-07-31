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

const splash = document.getElementById("splash");
if (splash) {
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      splash.dataset.hidden = "true";
      const remove = () => splash.remove();
      splash.addEventListener("transitionend", remove, { once: true });
      window.setTimeout(remove, 600);
    }),
  );
}

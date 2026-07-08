// Tema do TYBA: dark é o canônico; light é tema completo dos tokens.
// Escolha do usuário (localStorage) > padrão dark. "system" segue o SO.
// Aplicação: [data-theme="light"] no <html>; sem atributo = dark.
// O terminal (xterm) permanece dark em qualquer tema — terminal é dark.

const STORAGE_KEY = "tyba.theme";

export type ThemeMode = "dark" | "light" | "system";

export const THEMES: ThemeMode[] = ["dark", "light", "system"];

const media = window.matchMedia("(prefers-color-scheme: light)");

let mode: ThemeMode = (() => {
  const saved = localStorage.getItem(STORAGE_KEY);
  return saved === "light" || saved === "system" ? saved : "dark";
})();

function apply() {
  const light = mode === "light" || (mode === "system" && media.matches);
  if (light) {
    document.documentElement.setAttribute("data-theme", "light");
  } else {
    document.documentElement.removeAttribute("data-theme");
  }
}

media.addEventListener("change", () => {
  if (mode === "system") apply();
});

export function getThemeMode(): ThemeMode {
  return mode;
}

export function setThemeMode(next: ThemeMode) {
  mode = next;
  localStorage.setItem(STORAGE_KEY, next);
  apply();
}

// aplica no import, antes do primeiro paint do React
apply();

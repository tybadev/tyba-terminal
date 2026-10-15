// Tema salvo no splash, antes de pintar. O core injeta `window.__TYBA_SPLASH__`
// ao criar a janela (src-tauri/src/splash.rs): o modo e as cores de cada slot,
// só hex validado. Arquivo próprio no <head> porque a CSP não aceita script
// inline. Sem dado, vale o `splash.css` puro (aparência do sistema).
(function () {
  if (window.__TYBA_DEV__ === true) {
    document.documentElement.setAttribute("data-dev", "");
  }
  var boot = window.__TYBA_SPLASH__;
  if (!boot || typeof boot !== "object") return;
  var light =
    boot.mode === "light" ||
    (boot.mode === "system" &&
      window.matchMedia("(prefers-color-scheme: light)").matches);
  var root = document.documentElement;
  root.setAttribute("data-boot-base", light ? "light" : "dark");
  root.style.colorScheme = light ? "light" : "dark";
  var palette = light ? boot.light : boot.dark;
  if (!palette) return;
  var hex = /^#[0-9a-f]{3,8}$/i;
  var keys = ["bg", "fg", "muted", "green", "cyan", "violet"];
  for (var i = 0; i < keys.length; i++) {
    var value = palette[keys[i]];
    if (typeof value === "string" && hex.test(value)) {
      root.style.setProperty("--" + keys[i], value);
    }
  }
})();

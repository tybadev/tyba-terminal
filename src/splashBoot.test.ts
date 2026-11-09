import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Roda o public/splash-boot.js de verdade contra um DOM mínimo de mentira.
const SOURCE = readFileSync(join(import.meta.dir, "..", "public", "splash-boot.js"), "utf8");

function boot(opts: { saved?: string | null; system?: string; storageThrows?: boolean }) {
  const status = { textContent: "restaurando sessões…" };
  const bar = { label: "Carregando", setAttribute: (_: string, v: string) => (bar.label = v) };
  const listeners: Array<() => void> = [];
  const root = { lang: "pt-BR", setAttribute() {}, style: { setProperty() {} } };
  const document = {
    documentElement: root,
    addEventListener: (_: string, fn: () => void) => listeners.push(fn),
    querySelector: (sel: string) => (sel === ".status" ? status : sel === ".filament" ? bar : null),
  };
  const window = {
    navigator: { language: opts.system },
    get localStorage() {
      if (opts.storageThrows) throw new Error("blocked");
      return { getItem: () => opts.saved ?? null };
    },
  };
  new Function("window", "document", SOURCE)(window, document);
  for (const fn of listeners) fn();
  return { lang: root.lang, status: status.textContent, label: bar.label };
}

describe("splash-boot: idioma", () => {
  it("usa a escolha salva antes do idioma do sistema", () => {
    expect(boot({ saved: "en", system: "pt-BR" })).toEqual({
      lang: "en",
      status: "restoring sessions…",
      label: "Loading",
    });
    expect(boot({ saved: "pt-BR", system: "en-US" }).status).toBe("restaurando sessões…");
  });

  it("sem escolha, segue o sistema como o src/i18n", () => {
    expect(boot({ system: "en-US" }).lang).toBe("en");
    expect(boot({ system: "pt-PT" }).status).toBe("restaurando sessões…");
    expect(boot({ saved: "fr", system: "de-DE" }).status).toBe("restoring sessions…");
  });

  it("sem dado nenhum, fica no pt-BR do HTML", () => {
    expect(boot({ storageThrows: true })).toEqual({
      lang: "pt-BR",
      status: "restaurando sessões…",
      label: "Carregando",
    });
  });
});

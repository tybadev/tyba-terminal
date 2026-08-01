import { describe, expect, it } from "bun:test";

import {
  buildMenuSpec,
  isMenuExtraId,
  toAccelerator,
  type MenuItemSpec,
} from "./appMenu";
import { MAC_BINDINGS } from "./keys";

const t = (key: string) => key;

function items(spec = buildMenuSpec(t, MAC_BINDINGS)): MenuItemSpec[] {
  return spec.submenus.flatMap((submenu) => submenu.items);
}

describe("toAccelerator", () => {
  it("traduz modificadores para o vocabulário do muda", () => {
    expect(toAccelerator("meta+p")).toBe("Cmd+P");
    expect(toAccelerator("meta+shift+t")).toBe("Cmd+Shift+T");
    expect(toAccelerator("ctrl+alt+shift+n")).toBe("Ctrl+Alt+Shift+N");
  });

  it("preserva teclas que não são letra", () => {
    expect(toAccelerator("meta+,")).toBe("Cmd+,");
    expect(toAccelerator("meta+]")).toBe("Cmd+]");
    expect(toAccelerator("meta+shift+arrowup")).toBe("Cmd+Shift+arrowup");
  });

  it("recusa combo sem modificador ou só com modificador", () => {
    expect(toAccelerator("p")).toBeNull();
    expect(toAccelerator("meta+shift")).toBeNull();
    expect(toAccelerator("")).toBeNull();
  });

  it("recusa modificador desconhecido", () => {
    expect(toAccelerator("hyper+p")).toBeNull();
  });
});

describe("buildMenuSpec", () => {
  it("não deixa ação de terminal propor acelerador", () => {
    // No macOS o AppKit consome o acelerador antes do webview: ⌘C de menu
    // desligaria o copiar do xterm. O core também barra, isto é a primeira
    // camada.
    const terminal = items().filter(
      (item) =>
        item.kind === "action" &&
        ["copy", "paste", "search", "selectAll"].includes(item.id),
    );
    expect(terminal.length).toBeGreaterThan(0);
    for (const item of terminal) {
      expect(item).not.toHaveProperty("accelerator");
    }
  });

  it("propaga o atalho do usuário para as ações que podem ter acelerador", () => {
    const palette = items().find(
      (item) => item.kind === "action" && item.id === "paletteActions",
    );
    expect(palette).toMatchObject({ accelerator: "Cmd+P" });
  });

  it("reflete rebind do usuário em vez do padrão", () => {
    const custom = { ...MAC_BINDINGS, newTab: "meta+shift+k" };
    const newTab = items(buildMenuSpec(t, custom)).find(
      (item) => item.kind === "action" && item.id === "newTab",
    );
    expect(newTab).toMatchObject({ accelerator: "Cmd+Shift+K" });
  });

  it("todo id extra é reconhecido pelo despachante", () => {
    const extras = items().filter(
      (item) => item.kind === "action" && item.id.startsWith("menu:"),
    );
    expect(extras.length).toBeGreaterThan(0);
    for (const item of extras) {
      if (item.kind === "action") expect(isMenuExtraId(item.id)).toBe(true);
    }
  });

  it("mantém os itens predefinidos de Editar — clipboard do webview depende deles", () => {
    const edit = buildMenuSpec(t, MAC_BINDINGS).submenus.find(
      (submenu) => submenu.label === "menuEdit",
    );
    const predefined = edit?.items
      .filter((item) => item.kind === "predefined")
      .map((item) => (item.kind === "predefined" ? item.name : ""));
    expect(predefined).toEqual([
      "undo",
      "redo",
      "cut",
      "copy",
      "paste",
      "select_all",
    ]);
  });

  it("não traz nenhum item do Tauri no Ajuda", () => {
    const help = buildMenuSpec(t, MAC_BINDINGS).submenus.at(-1);
    expect(help?.label).toBe("menuHelp");
    expect(help?.items.every((item) => item.kind === "action")).toBe(true);
  });
});

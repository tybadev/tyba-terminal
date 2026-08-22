import { describe, expect, it } from "bun:test";

import { nextPromptMode } from "./promptModePicker";

describe("nextPromptMode", () => {
  it("as quatro setas andam, não só as horizontais", () => {
    // Os cartões ficam lado a lado em tela larga e EMPILHADOS em tela estreita.
    // Tratar só ←/→ deixaria o seletor mudo justamente no layout em que a seta
    // natural é ↑/↓.
    for (const key of ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"]) {
      expect(nextPromptMode(true, key)).toBe(false);
      expect(nextPromptMode(false, key)).toBe(true);
    }
  });

  it("Home e End são absolutos, e é o que os separa das setas", () => {
    // Com duas opções, circular é o mesmo que alternar — então uma seta pode
    // ser escrita como `!current`. `Home`/`End` não: eles vão para a primeira e
    // para a última, e escrevê-los como alternância faria o destino depender da
    // seleção atual, que é o oposto do que significam.
    expect(nextPromptMode(true, "Home")).toBe(true);
    expect(nextPromptMode(false, "Home")).toBe(true);
    expect(nextPromptMode(true, "End")).toBe(false);
    expect(nextPromptMode(false, "End")).toBe(false);
  });

  it("tecla que não é do grupo devolve null, e não uma opção", () => {
    // `null` é o que faz o componente NÃO chamar `preventDefault`. Devolver um
    // booleano aqui engoliria o Tab e prenderia o foco dentro do seletor.
    for (const key of ["Tab", "Enter", " ", "Escape", "a", "PageDown"]) {
      expect(nextPromptMode(true, key)).toBeNull();
      expect(nextPromptMode(false, key)).toBeNull();
    }
  });
});

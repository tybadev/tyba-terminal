import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { FocusHUD } from "./focus-hud";

const noop = () => {};

describe("FocusHUD", () => {
  test("rodando: ponto pulsando, tempo, tarefa, pausar e encerrar", () => {
    const html = renderToStaticMarkup(
      <FocusHUD time="01:12" task="claude · handoff da aba Turno" onPause={noop} onStop={noop} />,
    );
    expect(html).toContain('data-state="running"');
    expect(html).toContain("Foco em andamento");
    expect(html).toContain("is-running");
    expect(html).toContain("01:12");
    expect(html).toContain("claude · handoff da aba Turno");
    expect(html).toContain('aria-label="Pausar"');
    expect(html).toContain('aria-label="Encerrar foco"');
    expect(html).toContain('data-variant="outline"');
  });

  test("pausado: sem pulso, tempo apagado e continuar no primário", () => {
    const html = renderToStaticMarkup(
      <FocusHUD time="01:12" paused onPause={noop} onStop={noop} />,
    );
    expect(html).toContain('data-state="paused"');
    expect(html).toContain("Foco pausado");
    expect(html).not.toContain("is-running");
    expect(html).toContain("text-tyba-text-muted");
    expect(html).toContain('aria-label="Continuar"');
    expect(html).toContain('data-variant="default"');
  });

  test("sem onPause (Windows): só encerrar; sem tarefa, sem o texto", () => {
    const html = renderToStaticMarkup(<FocusHUD time="00:03" onStop={noop} />);
    expect(html).not.toContain("Pausar");
    expect(html).toContain("Encerrar foco");
    expect(html).not.toContain("max-w-[200px]");
  });
});

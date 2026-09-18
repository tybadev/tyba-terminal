import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { Input } from "./input";
import { Textarea } from "./textarea";

// Regra 29: o macOS com "Capitalizar palavras automaticamente" ligado fez o
// WebKit gravar um Host com usuário `Root`. Campo técnico nasce sem correção.
describe("campos base não corrigem o que se digita", () => {
  test("Input sai sem capitalização, correção e verificação ortográfica", () => {
    const html = renderToStaticMarkup(<Input />).toLowerCase();
    expect(html).toContain('autocapitalize="off"');
    expect(html).toContain('autocorrect="off"');
    expect(html).toContain('spellcheck="false"');
  });

  test("Textarea sai sem capitalização, correção e verificação ortográfica", () => {
    const html = renderToStaticMarkup(<Textarea />).toLowerCase();
    expect(html).toContain('autocapitalize="off"');
    expect(html).toContain('autocorrect="off"');
    expect(html).toContain('spellcheck="false"');
  });

  test("campo de prosa religa a correção explicitamente", () => {
    for (const html of [
      renderToStaticMarkup(
        <Input autoCapitalize="sentences" autoCorrect="on" spellCheck />,
      ),
      renderToStaticMarkup(
        <Textarea autoCapitalize="sentences" autoCorrect="on" spellCheck />,
      ),
    ].map((h) => h.toLowerCase())) {
      expect(html).toContain('autocapitalize="sentences"');
      expect(html).toContain('autocorrect="on"');
      expect(html).toContain('spellcheck="true"');
      expect(html).not.toContain('autocapitalize="off"');
    }
  });
});

import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { Dialog as DialogPrimitive } from "radix-ui";

import { TooltipProvider } from "@/components/ui/tooltip";
import type { HandoffTarget, TurnDigest } from "../lib/ipc";
import { HandoffSheetBody } from "./HandoffSheet";
import { TurnPanel } from "./TurnPanel";

const turn: TurnDigest = {
  seq: 1,
  outcome: "completed",
  failure: null,
  started_at_ms: 0,
  ended_at_ms: 1,
  duration_ms: 1,
  prompt: "fecha o bug do OSC 133",
  diff: "ok",
  files: [{ path: "src/osc133.rs", old_path: null, status: "modified", insertions: 12, deletions: 3 }],
  files_total: 1,
  insertions: 12,
  deletions: 3,
  commits: 1,
} as TurnDigest;

const TARGETS: HandoffTarget[] = [
  { agent: "codex", label: "Codex", available: true, reason: null },
  { agent: "gemini", label: "Gemini", available: false, reason: "sem suporte ainda" },
];

const noop = () => {};

function panel(turnOpen: boolean, handoff = true) {
  return renderToStaticMarkup(
    <TooltipProvider>
      <TurnPanel sessionId="s1" turn={turn} turnOpen={turnOpen} onHandoff={handoff ? noop : undefined} />
    </TooltipProvider>,
  );
}

type SheetProps = Parameters<typeof HandoffSheetBody>[0];

function sheet(preview: SheetProps["preview"], extra: Partial<SheetProps> = {}) {
  // O corpo usa DialogTitle/DialogDescription: precisam do Root em volta.
  return renderToStaticMarkup(
    <DialogPrimitive.Root>
      <HandoffSheetBody
        from="claude"
        turn={turn}
        targets={TARGETS}
        target="codex"
        onTarget={noop}
        preview={preview}
        previewNote="tira o TODO"
        error={null}
        note="tira o TODO"
        onNote={noop}
        opening={false}
        onCancel={noop}
        onOpen={noop}
        {...extra}
      />
    </DialogPrimitive.Root>,
  );
}

describe("TurnPanel: Passar para… (H1)", () => {
  test("turno fechado: ligado, ao lado de Ver diff do turno", () => {
    const html = panel(false);
    const diff = html.indexOf("turnSeeDiff");
    const pass = html.indexOf("handoffPassTo");
    expect(diff).toBeGreaterThan(-1);
    expect(pass).toBeGreaterThan(diff);
    expect(html).not.toMatch(/<button[^>]*disabled=""[^>]*>(?:(?!<\/button>).)*handoffPassTo/);
  });

  test("turno aberto: desligado", () => {
    const html = panel(true);
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>(?:(?!<\/button>).)*handoffPassTo/);
  });

  test("runner custom (sem onHandoff): o botão não aparece", () => {
    const html = panel(false, false);
    expect(html).toContain("turnSeeDiff");
    expect(html).not.toContain("handoffPassTo");
  });

  test("o rodapé desde HH:MM fica inteiro e a linha quebra", () => {
    const html = panel(false);
    expect(html).toContain("flex-wrap");
    expect(html).toMatch(/whitespace-nowrap text-\[11px\] text-tyba-text-faint">turnSince/);
  });
});

describe("HandoffSheetBody (H2)", () => {
  const preview = {
    text: "Handoff de Claude · tyba-terminal\n\nPedido:\nfecha o bug\nGH_TOKEN=[REDACTED]\n\nO que o Claude disse por último:\n> Feito:\n\nNota do dono (o que fazer agora):\ntira o TODO\n",
    body: "Handoff de Claude · tyba-terminal\n\nPedido:\nfecha o bug\nGH_TOKEN=[REDACTED]\n\nO que o Claude disse por último:\n> Feito:\n",
    note: "tira o TODO",
    redacted: 1,
    folder: "tyba-terminal",
    target_label: "Codex",
  };

  test("Gemini indisponível: apagado, com o motivo do core", () => {
    const html = sheet(preview);
    expect(html).toMatch(/<button[^>]*aria-checked="false"[^>]*disabled=""[^>]*>.*?Gemini.*?sem suporte ainda/);
    expect(html).toMatch(/<button[^>]*aria-checked="true"[^>]*>.*?Codex.*?handoffInstalled/);
  });

  test("mostra o texto do core, a redação e o campo da nota", () => {
    const html = sheet(preview);
    expect(html).toContain("handoffRedacted");
    expect(html).toContain("[REDACTED]");
    expect(html).toContain("handoffNoteLabel");
    expect(html).toContain("tira o TODO</textarea>");
    // A folha desenha o body: a nota não aparece duas vezes.
    expect(html).not.toContain("Nota do dono");
    expect(html).toContain("handoffNoApprovals");
    expect(html).toContain("handoffSameFolder");
    expect(html).toContain("handoffOpenIn");
  });

  test("desenha o body do core; a linha citada fica apagada, sem destaque", () => {
    const html = sheet(preview);
    expect(html).toContain("fecha o bug");
    expect(html).toContain('<span class="text-tyba-text-faint">&gt; Feito:');
    expect(html).not.toMatch(/font-medium text-tyba-text">&gt; Feito:/);
    expect(html).toContain('font-medium text-tyba-text">O que o Claude disse por último:');
  });

  test("a nota tem teto de 2000 no campo", () => {
    expect(sheet(preview)).toMatch(/<textarea[^>]*maxLength="2000"/);
  });

  test("nota igual à digitada: sem Vai assim", () => {
    expect(sheet(preview)).not.toContain("handoffNoteAsSent");
  });

  test("o core mudou a nota: Vai assim com a nota como vai e o chip de redação", () => {
    const html = sheet(
      { ...preview, note: "usa [REDACTED] no teste" },
      { note: "usa sk-abc no teste", previewNote: "usa sk-abc no teste" },
    );
    expect(html).toMatch(
      /data-testid="handoff-note-as-sent"[^>]*>handoffNoteAsSent usa <span title="handoffRedactedMark"[^>]*>.*?\[REDACTED\]<\/span> no teste/,
    );
  });

  test("erro do core: fica dentro da folha no lugar do texto, Abrir desligado", () => {
    const html = sheet(null, { error: "A pasta não é um repositório git; o handoff precisa de um." });
    expect(html).toContain('role="alert"');
    expect(html).toContain("A pasta não é um repositório git");
    expect(html).not.toContain('data-testid="handoff-preview"');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>[^<]*(<svg.*?<\/svg>)?handoffOpenIn/);
  });

  test("sem destino disponível: Nenhum agente disponível, desligado", () => {
    const html = sheet(null, {
      targets: [{ agent: "gemini", label: "Gemini", available: false, reason: "sem suporte ainda" }],
      target: null,
    });
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>handoffNoAgent<\/button>/);
    expect(html).not.toContain("handoffOpenIn");
  });

  test("sem texto do core ainda: Abrir desligado", () => {
    const html = sheet(null);
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>[^<]*(<svg.*?<\/svg>)?handoffOpenIn/);
  });
});

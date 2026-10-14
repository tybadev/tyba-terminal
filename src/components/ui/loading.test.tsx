import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { LoadingState } from "./loading";

const base = { step: "lendo ~/poc-git", em: ["~/poc-git"], op: "files" as const };

describe("LoadingState", () => {
  test("trabalhando: filamento vivo, esqueleto e a etapa com o fato aceso", () => {
    const html = renderToStaticMarkup(<LoadingState {...base} startedAt={Date.now()} shape="tree" />);
    expect(html).toContain("tyba-flow-line--live");
    expect(html).toContain("tyba-skeleton");
    expect(html).toContain('<span class="text-tyba-text">~/poc-git</span>');
    expect(html).not.toContain("loadingSlow");
    expect(html).not.toContain("tyba-load-bar");
  });

  test("passou do limiar: o motivo e a ação", () => {
    const html = renderToStaticMarkup(
      <LoadingState
        {...base}
        startedAt={Date.now() - 60_000}
        slowCause="pasta grande ou disco lento."
        slowAction={{ label: "Abrir", onClick: () => {} }}
      />,
    );
    expect(html).toContain("loadingSlow");
    expect(html).toContain("pasta grande ou disco lento.");
    expect(html).toContain("Abrir");
  });

  test("falhou: sem filamento vivo, sem esqueleto, erro e tentar de novo", () => {
    const html = renderToStaticMarkup(
      <LoadingState {...base} startedAt={0} error="git: not a repository" onRetry={() => {}} />,
    );
    expect(html).not.toContain("tyba-flow-line--live");
    expect(html).not.toContain("tyba-skeleton");
    expect(html).toContain("git: not a repository");
    expect(html).toContain("retry");
  });

  test("barra de progresso só com total conhecido", () => {
    const sem = renderToStaticMarkup(<LoadingState {...base} startedAt={Date.now()} done={3} />);
    expect(sem).not.toContain("tyba-load-bar");
    const com = renderToStaticMarkup(
      <LoadingState {...base} startedAt={Date.now()} done={1} total={4} />,
    );
    expect(com).toContain("tyba-load-bar");
    expect(com).toContain("width:25%");
  });

  test("sem forma: só filamento e etapa", () => {
    const html = renderToStaticMarkup(<LoadingState {...base} startedAt={Date.now()} shape={null} />);
    expect(html).not.toContain("tyba-skeleton");
    expect(html).toContain("tyba-flow-line--live");
  });
});

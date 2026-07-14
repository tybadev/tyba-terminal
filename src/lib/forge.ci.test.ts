import { describe, expect, it } from "bun:test";

import { hasRunningWork, jobTone, runTone, sortRuns } from "./forge";
import type { WorkflowRun } from "./ipc";

function run(over: Partial<WorkflowRun>): WorkflowRun {
  return {
    id: 1,
    name: "CI",
    status: "completed",
    conclusion: "success",
    url: "https://example.test/1",
    head_branch: "main",
    event: "push",
    created_at: "2026-07-14T05:00:00Z",
    ...over,
  };
}

describe("runTone", () => {
  it("run rodando é pendente, mesmo sem conclusão", () => {
    expect(runTone({ status: "in_progress", conclusion: null })).toBe("pending");
    expect(runTone({ status: "queued", conclusion: null })).toBe("pending");
  });

  it("sucesso, skipped e neutral contam como verde", () => {
    for (const conclusion of ["success", "skipped", "neutral"]) {
      expect(runTone({ status: "completed", conclusion })).toBe("success");
    }
  });

  it("qualquer outra conclusão é falha — inclusive uma que o GitHub invente", () => {
    for (const conclusion of ["failure", "cancelled", "timed_out", "sei_la"]) {
      expect(runTone({ status: "completed", conclusion })).toBe("failure");
    }
  });

  it("concluído sem conclusão não vira verde por engano", () => {
    expect(runTone({ status: "completed", conclusion: null })).toBe("pending");
  });
});

describe("hasRunningWork", () => {
  it("é o que decide se o painel continua consultando", () => {
    expect(hasRunningWork([run({}), run({ id: 2 })])).toBe(false);
    expect(hasRunningWork([run({}), run({ id: 2, status: "in_progress" })])).toBe(
      true,
    );
    expect(hasRunningWork([])).toBe(false);
  });
});

describe("sortRuns", () => {
  it("o que está rodando vem antes do que já terminou", () => {
    const antigoRodando = run({
      id: 1,
      status: "in_progress",
      conclusion: null,
      created_at: "2026-07-14T04:00:00Z",
    });
    const novoConcluido = run({ id: 2, created_at: "2026-07-14T06:00:00Z" });
    expect(sortRuns([novoConcluido, antigoRodando]).map((r) => r.id)).toEqual([
      1, 2,
    ]);
  });

  it("entre concluídos, o mais recente primeiro", () => {
    const velho = run({ id: 1, created_at: "2026-07-14T04:00:00Z" });
    const novo = run({ id: 2, created_at: "2026-07-14T06:00:00Z" });
    expect(sortRuns([velho, novo]).map((r) => r.id)).toEqual([2, 1]);
  });

  it("não muta a lista original", () => {
    const runs = [run({ id: 1 }), run({ id: 2, status: "in_progress" })];
    sortRuns(runs);
    expect(runs.map((r) => r.id)).toEqual([1, 2]);
  });
});

describe("jobTone", () => {
  it("job na fila é pendente e não tem url — a UI não pode explodir", () => {
    expect(
      jobTone({ name: "publish", status: "queued", conclusion: null, url: null }),
    ).toBe("pending");
  });
});

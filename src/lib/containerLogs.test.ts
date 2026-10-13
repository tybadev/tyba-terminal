import { describe, expect, it } from "bun:test";

import {
  appendCapped,
  compactCpu,
  compactMem,
  countMatches,
  exportLine,
  formatRunningFor,
  highlightParts,
  levelTotals,
  LOG_LINES_CAP,
  ownTimestamp,
  sinceLabel,
  visibleLogs,
} from "./containerLogs";
import type { LogLine } from "./ipc";

const NOW = Date.parse("2026-10-12T15:30:00Z");

function line(
  seq: number,
  text: string,
  opts: Partial<LogLine> = {},
): LogLine {
  return {
    seq,
    ts: null,
    ts_ms: null,
    level: null,
    continuation: false,
    stderr: false,
    text,
    frame: null,
    ...opts,
  };
}

const at = (iso: string) => ({ ts: iso, ts_ms: Date.parse(iso) });

const SAMPLE: LogLine[] = [
  line(1, "Servidor em :3000", { ...at("2026-10-12T15:00:00Z"), level: "info" }),
  line(2, "GET /health 200 1ms", at("2026-10-12T15:24:11Z")),
  line(3, "Pool do Postgres a 80%", { ...at("2026-10-12T15:25:40Z"), level: "warn" }),
  line(4, "TypeError: process.uptim is not a function", {
    ...at("2026-10-12T15:26:58Z"),
    level: "error",
  }),
  line(5, "    at /app/src/server.js:8:43", {
    level: "error",
    continuation: true,
    frame: { path: "/app/src/server.js", line: 8, col: 43, host_path: null },
  }),
  line(6, "GET /health 500 3ms", at("2026-10-12T15:26:58Z")),
];

describe("visibleLogs", () => {
  it("chip Erros mostra o erro com o trace junto", () => {
    expect(visibleLogs(SAMPLE, "errors", "all", NOW).map((l) => l.seq)).toEqual([4, 5]);
    expect(visibleLogs(SAMPLE, "warnings", "all", NOW).map((l) => l.seq)).toEqual([3]);
  });

  it("janela de tempo usa a hora da última linha datada para as continuações", () => {
    const last15 = visibleLogs(SAMPLE, "all", "15m", NOW).map((l) => l.seq);
    expect(last15).toEqual([2, 3, 4, 5, 6]);
    expect(visibleLogs(SAMPLE, "all", "5m", NOW).map((l) => l.seq)).toEqual([3, 4, 5, 6]);
  });
});

describe("filtro acende", () => {
  it("conta casamentos sem diferenciar caixa", () => {
    expect(countMatches(SAMPLE, "HEALTH")).toBe(2);
    expect(countMatches(SAMPLE, "  ")).toBe(0);
  });

  it("marca cada ocorrência", () => {
    expect(highlightParts("GET /health e /health", "health")).toEqual([
      { text: "GET /", hit: false },
      { text: "health", hit: true },
      { text: " e /", hit: false },
      { text: "health", hit: true },
    ]);
    expect(highlightParts("nada", "")).toEqual([{ text: "nada", hit: false }]);
  });
});

describe("totais e exportação", () => {
  it("conta eventos, não linhas de trace", () => {
    expect(levelTotals(SAMPLE)).toEqual({ errors: 1, warnings: 1 });
  });

  it("linha exportada leva a hora crua do docker quando pedida", () => {
    expect(exportLine(SAMPLE[1], true)).toBe("2026-10-12T15:24:11Z GET /health 200 1ms");
    expect(exportLine(SAMPLE[1], false)).toBe("GET /health 200 1ms");
  });

  it("buffer da tela tem teto", () => {
    const many = Array.from({ length: LOG_LINES_CAP }, (_, i) => line(i, "x"));
    const next = appendCapped(many, [line(LOG_LINES_CAP, "y")]);
    expect(next.length).toBe(LOG_LINES_CAP);
    expect(next[next.length - 1].text).toBe("y");
    expect(next[0].seq).toBe(1);
  });

  it("minutos desde que subiu", () => {
    expect(sinceLabel("2026-10-12T15:27:00Z", NOW)).toBe(3);
    expect(sinceLabel(null, NOW)).toBeNull();
    expect(sinceLabel("lixo", NOW)).toBeNull();
  });
});

describe("formatRunningFor", () => {
  it("fala em minutos, horas e dias, nunca em minutos crus", () => {
    expect(formatRunningFor(0)).toBe("<1 min");
    expect(formatRunningFor(25)).toBe("25 min");
    expect(formatRunningFor(60)).toBe("1 h");
    expect(formatRunningFor(125)).toBe("2 h 5 min");
    expect(formatRunningFor(1537)).toBe("1 d 1 h");
    expect(formatRunningFor(2880)).toBe("2 d");
  });
});

describe("compactCpu / compactMem", () => {
  it("arredonda a CPU e encurta a memória", () => {
    expect(compactCpu("2.13%")).toBe("2%");
    expect(compactCpu("--")).toBe("--");
    expect(compactMem("358.2MiB")).toBe("358 MB");
    expect(compactMem("1.52GiB")).toBe("1.5 GB");
    expect(compactMem("512KiB")).toBe("512 KB");
    expect(compactMem("0B")).toBe("0 B");
    expect(compactMem("??")).toBe("??");
  });
});

describe("ownTimestamp", () => {
  it("reconhece ISO, com e sem zona, e tira o prefixo da mensagem", () => {
    expect(ownTimestamp("2026-10-12T15:24:02.123Z GET /health 200")).toEqual({
      clock: "15:24:02",
      rest: "GET /health 200",
    });
    expect(ownTimestamp("2026-10-12 15:24:02,481 - api - INFO - pronto")).toEqual({
      clock: "15:24:02",
      rest: "api - INFO - pronto",
    });
    expect(ownTimestamp("[2026-10-12T15:24:02+00:00] boot")).toEqual({
      clock: "15:24:02",
      rest: "boot",
    });
  });

  it("reconhece postgres, nginx, syslog e [HH:MM:SS]", () => {
    expect(
      ownTimestamp("2026-10-12 15:24:02.481 UTC [1] LOG:  database system is ready"),
    ).toEqual({ clock: "15:24:02", rest: "[1] LOG:  database system is ready" });
    expect(ownTimestamp("2026/10/12 15:24:02 [error] 29#29: upstream")?.clock).toBe(
      "15:24:02",
    );
    expect(ownTimestamp("Oct  2 09:05:01 web cron[12]: tick")).toEqual({
      clock: "09:05:01",
      rest: "web cron[12]: tick",
    });
    expect(ownTimestamp("[09:05:01] compiled")).toEqual({ clock: "09:05:01", rest: "compiled" });
  });

  it("não confunde nível com fuso nem pega hora no meio da linha", () => {
    expect(ownTimestamp("2026-10-12 15:24:02 INFO pronto")?.rest).toBe("INFO pronto");
    expect(ownTimestamp("GET /health 200 em 2026-10-12T15:24:02Z")).toBeNull();
    expect(ownTimestamp("2026-10-12T15:24:02Zlixo")).toBeNull();
    expect(ownTimestamp("")).toBeNull();
  });
});

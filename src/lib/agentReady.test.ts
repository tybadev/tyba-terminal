import { describe, expect, test } from "bun:test";

import { scheduleAgentReadyPrompt } from "./agentReady";

interface FakeTimers {
  setTimeout: (cb: () => void, ms: number) => number;
  clearTimeout: (handle: number) => void;
  fire: (handle: number) => void;
  cleared: Set<number>;
}

function fakeTimers(): FakeTimers {
  let nextHandle = 1;
  const pending = new Map<number, () => void>();
  const cleared = new Set<number>();
  return {
    setTimeout: (cb, _ms) => {
      const handle = nextHandle++;
      pending.set(handle, cb);
      return handle;
    },
    clearTimeout: (handle) => {
      cleared.add(handle);
      pending.delete(handle);
    },
    fire: (handle) => {
      pending.get(handle)?.();
    },
    cleared,
  };
}

describe("scheduleAgentReadyPrompt", () => {
  test("ready antes do timeout: cola e submete, cancela o timeout", () => {
    const timers = fakeTimers();
    const pasted: boolean[] = [];
    let timedOut = false;
    const ready: { handler: (() => void) | null } = { handler: null };
    let unsubscribed = false;

    scheduleAgentReadyPrompt({
      onReady: (handler) => {
        ready.handler = handler;
        return () => {
          unsubscribed = true;
        };
      },
      paste: (submit) => pasted.push(submit),
      onTimeout: () => {
        timedOut = true;
      },
      setTimeout: timers.setTimeout,
      clearTimeout: timers.clearTimeout,
    });

    ready.handler?.();

    expect(pasted).toEqual([true]);
    expect(timedOut).toBe(false);
    expect(unsubscribed).toBe(true);
    expect(timers.cleared.size).toBe(1);
  });

  test("timeout sem ready: NÃO cola com submit, só preenche e avisa", () => {
    const timers = fakeTimers();
    const pasted: boolean[] = [];
    let timedOut = false;
    let handle = 0;

    scheduleAgentReadyPrompt({
      onReady: () => () => {},
      paste: (submit) => pasted.push(submit),
      onTimeout: () => {
        timedOut = true;
      },
      setTimeout: (cb, ms) => {
        handle = timers.setTimeout(cb, ms);
        return handle;
      },
      clearTimeout: timers.clearTimeout,
    });

    timers.fire(handle);

    expect(pasted).toEqual([false]);
    expect(timedOut).toBe(true);
  });

  test("ready depois do timeout já disparado não cola de novo", () => {
    const timers = fakeTimers();
    const pasted: boolean[] = [];
    const ready: { handler: (() => void) | null } = { handler: null };
    let handle = 0;

    scheduleAgentReadyPrompt({
      onReady: (handler) => {
        ready.handler = handler;
        return () => {};
      },
      paste: (submit) => pasted.push(submit),
      onTimeout: () => {},
      setTimeout: (cb, ms) => {
        handle = timers.setTimeout(cb, ms);
        return handle;
      },
      clearTimeout: timers.clearTimeout,
    });

    timers.fire(handle);
    ready.handler?.();

    expect(pasted).toEqual([false]);
  });

  test("cancelar antes de qualquer evento não cola nada", () => {
    const timers = fakeTimers();
    const pasted: boolean[] = [];
    let unsubscribed = false;

    const cancel = scheduleAgentReadyPrompt({
      onReady: () => () => {
        unsubscribed = true;
      },
      paste: (submit) => pasted.push(submit),
      onTimeout: () => {},
      setTimeout: timers.setTimeout,
      clearTimeout: timers.clearTimeout,
    });

    cancel();

    expect(pasted).toEqual([]);
    expect(unsubscribed).toBe(true);
    expect(timers.cleared.size).toBe(1);
  });
});

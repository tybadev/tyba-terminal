import { describe, expect, test } from "bun:test";

import { dayMonth, targetLabel } from "./typedSsh";

describe("targetLabel", () => {
  test("user@host:porta, como o core", () => {
    expect(targetLabel({ user: "root", host: "203.0.113.10", port: 22 })).toBe("root@203.0.113.10:22");
    expect(targetLabel({ user: null, host: "web.example.test", port: 2222 })).toBe("web.example.test:2222");
  });
  test("IPv6 entre colchetes", () => {
    expect(targetLabel({ user: "me", host: "2001:db8::1", port: 2200 })).toBe("me@[2001:db8::1]:2200");
  });
});

describe("dayMonth", () => {
  test("dd/mm na hora local", () => {
    expect(dayMonth(new Date(2026, 10, 9, 14, 2).getTime())).toBe("09/11");
    expect(dayMonth(new Date(2026, 0, 28).getTime())).toBe("28/01");
  });
});

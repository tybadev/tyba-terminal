import { describe, expect, test } from "bun:test";

import { agentBinaryName } from "./shellAgentNotice";

describe("agentBinaryName", () => {
  test("mapeia runners conhecidos e custom", () => {
    expect(agentBinaryName("claude_code")).toBe("claude");
    expect(agentBinaryName("codex")).toBe("codex");
    expect(agentBinaryName({ custom: "gemini" })).toBe("gemini");
  });
});

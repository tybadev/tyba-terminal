import { describe, expect, test } from "bun:test"

import { ptyExitEndsSession } from "./sessionExit"

describe("ptyExitEndsSession", () => {
  test("SSH: o cano cair não encerra a sessão", () => {
    expect(ptyExitEndsSession({ type: "ssh", host_id: "h1" })).toBe(false)
  })

  test("shell local: o PTY é a sessão", () => {
    expect(ptyExitEndsSession({ type: "shell" })).toBe(true)
  })

  test("agente: o PTY é a sessão", () => {
    expect(ptyExitEndsSession({ type: "agent", runner: "claude_code" })).toBe(
      true,
    )
  })

  test("container: o PTY é a sessão", () => {
    expect(
      ptyExitEndsSession({ type: "container", host_id: null, container_id: "c" }),
    ).toBe(true)
  })
})

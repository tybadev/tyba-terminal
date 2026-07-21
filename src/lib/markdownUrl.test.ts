import { describe, expect, it } from "bun:test";

import { isDangerousUrl, isRemoteUrl, safeMarkdownUrl } from "./markdownUrl";

describe("isRemoteUrl", () => {
  it("flags http, https and protocol-relative urls", () => {
    expect(isRemoteUrl("http://evil/x.png")).toBe(true);
    expect(isRemoteUrl("https://evil/x.png")).toBe(true);
    expect(isRemoteUrl("//evil/x.png")).toBe(true);
    expect(isRemoteUrl("  HTTPS://EVIL/x.png  ")).toBe(true);
  });

  it("does not flag inline data urls or relative paths", () => {
    expect(isRemoteUrl("data:image/png;base64,AAAA")).toBe(false);
    expect(isRemoteUrl("./local.png")).toBe(false);
    expect(isRemoteUrl("images/local.png")).toBe(false);
  });
});

describe("safeMarkdownUrl", () => {
  it("blocks a remote image beacon", () => {
    expect(safeMarkdownUrl("http://evil/track.png")).toBe("");
  });

  it("blocks script and inline-html schemes", () => {
    expect(isDangerousUrl("javascript:alert(1)")).toBe(true);
    expect(safeMarkdownUrl("javascript:alert(1)")).toBe("");
    expect(safeMarkdownUrl("data:text/html,<script>x</script>")).toBe("");
  });

  it("keeps inline images and local links", () => {
    expect(safeMarkdownUrl("data:image/png;base64,AAAA")).toBe(
      "data:image/png;base64,AAAA",
    );
    expect(safeMarkdownUrl("./local.png")).toBe("./local.png");
    expect(safeMarkdownUrl("mailto:x@y.z")).toBe("mailto:x@y.z");
  });
});

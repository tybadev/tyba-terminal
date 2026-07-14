import { describe, expect, it } from "bun:test";

import { changelogUrl, siteLocale } from "./changelog";

describe("siteLocale", () => {
  it("mapeia o pt-BR da app pro pt-br do site", () => {
    expect(siteLocale("pt-BR")).toBe("pt-br");
    expect(siteLocale("pt-br")).toBe("pt-br");
  });

  it("cai no idioma base quando a região não bate", () => {
    expect(siteLocale("pt-PT")).toBe("pt-br");
    expect(siteLocale("en-US")).toBe("en");
  });

  it("cai no en quando o site não tem o idioma", () => {
    expect(siteLocale("fr")).toBe("en");
    expect(siteLocale("")).toBe("en");
  });
});

describe("changelogUrl", () => {
  it("aponta pro changelog do site, não pro release do GitHub", () => {
    expect(changelogUrl("pt-BR")).toBe("https://tyba.dev/pt-br/changelog");
    expect(changelogUrl("en")).toBe("https://tyba.dev/en/changelog");
  });

  it("nunca gera uma rota fora do domínio do site", () => {
    expect(changelogUrl("../evil")).toStartWith("https://tyba.dev/");
  });
});

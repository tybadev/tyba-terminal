import { describe, expect, it } from "bun:test";

import { agentOpenUrlToastInput } from "./agentOpenUrlToast";
import type { AgentOpenUrlPayload } from "./ipc";

const t = (key: string, options?: Record<string, unknown>) =>
  options ? `${key}(${JSON.stringify(options)})` : key;

const knownPayload: AgentOpenUrlPayload = {
  session_id: "s1",
  url: "https://claude.ai/oauth/authorize?client_id=attacker&redirect_uri=https%3A%2F%2Fevil.example%2Fcb",
  host: "claude.ai",
  known_login: true,
};

describe("agentOpenUrlToastInput", () => {
  // Review r1, MAJOR: o vetor de phishing exato -- host=claude.ai (known_login
  // true) mas a URL real carrega um redirect_uri de terceiro. Esconder a URL
  // nesse caso deixa o dono autorizar sem ver o redirect malicioso.
  it("known_login=true NUNCA esconde a url completa, mesmo com client_id/redirect_uri de terceiro", () => {
    const toast = agentOpenUrlToastInput(knownPayload, t);
    expect(toast.detail).toBe(knownPayload.url);
    expect(toast.detail).toContain("evil.example");
  });

  it("known_login=false também mostra a url completa", () => {
    const payload: AgentOpenUrlPayload = {
      ...knownPayload,
      host: "github.com",
      known_login: false,
      url: "https://github.com/login/oauth/authorize?client_id=x",
    };
    const toast = agentOpenUrlToastInput(payload, t);
    expect(toast.detail).toBe(payload.url);
  });

  it("o host real aparece no título, não um rótulo fixo mentiroso", () => {
    for (const host of [
      "claude.ai",
      "platform.claude.com",
      "console.anthropic.com",
      "anthropic.com",
    ]) {
      const toast = agentOpenUrlToastInput(
        { ...knownPayload, host, url: `https://${host}/authorize` },
        t,
      );
      expect(toast.title).toContain(host);
    }
  });

  // Item 34 do contrato de cobertura, reescrito pós-fix: o guarda-corpo de
  // regressão do phishing -- known_login muda SÓ a cópia (título), a ação
  // que de fato abre o navegador é idêntica nos dois casos, sempre com a
  // MESMA url do payload. Se algum dia known_login=true voltar a abrir algo
  // diferente da url real (ex.: um host "confiável" fixo), este teste
  // reprova.
  it("known_login muda só a cópia -- action.run é idêntico (mesma url) nos dois casos", () => {
    const calls: string[] = [];
    const openUrl = (url: string) => calls.push(url);

    const known = agentOpenUrlToastInput(knownPayload, t, openUrl);
    const unknown = agentOpenUrlToastInput(
      { ...knownPayload, known_login: false },
      t,
      openUrl,
    );

    known.action?.run();
    unknown.action?.run();

    expect(calls).toEqual([knownPayload.url, knownPayload.url]);
    expect(known.detail).toBe(unknown.detail);
    expect(known.action?.label).toBe(unknown.action?.label);
    expect(known.title).not.toBe(unknown.title);
  });
});

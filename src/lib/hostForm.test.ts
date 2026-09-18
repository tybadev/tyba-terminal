import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

import {
  agentKeyType,
  agentListingState,
  applyInputToHost,
  emptyHostForm,
  formToInput,
  hostToForm,
  usernameNeedsWarning,
  type HostFormValues,
} from "./hostForm";
import { EVENT_HOSTS_CHANGED, type Host } from "./ipc";

const host = (over: Partial<Host> = {}): Host => ({
  id: "h1",
  alias: "vps",
  hostname: "vps.example.test",
  port: null,
  username: "root",
  identity_file: null,
  proxy_jump: null,
  group_id: null,
  color: null,
  notes: null,
  position: 0,
  tunnels: [],
  auth_method: "auto",
  agent_key: null,
  created_at: "2026-09-16T00:00:00Z",
  last_connected_at: null,
  ...over,
});

const AGENT_KEY = {
  public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleExampleExampleExampleExample00 work",
  name: "work",
  fingerprint: "SHA256:exampleexampleexampleexampleexampleexample00",
};

describe("hostToForm", () => {
  // Regra 5: o formulário nunca produz `auto` com arquivo. Host legado com
  // caminho aparece como `file` e só muda de verdade se o dono salvar.
  test("host legado automático com arquivo abre como arquivo", () => {
    const form = hostToForm(host({ identity_file: "~/.ssh/id_ed25519" }));
    expect(form.auth_method).toBe("file");
    expect(form.identity_file).toBe("~/.ssh/id_ed25519");
  });

  test("host automático sem arquivo continua automático", () => {
    expect(hostToForm(host()).auth_method).toBe("auto");
  });

  test("host sem método declarado é automático", () => {
    const legacy = host();
    delete legacy.auth_method;
    delete legacy.agent_key;
    expect(hostToForm(legacy).auth_method).toBe("auto");
    expect(hostToForm(legacy).agent_key).toBeNull();
  });

  test("chave do agente escolhida volta ao formulário", () => {
    const form = hostToForm(host({ auth_method: "agent", agent_key: AGENT_KEY }));
    expect(form.auth_method).toBe("agent");
    expect(form.agent_key).toEqual(AGENT_KEY);
  });
});

describe("usernameNeedsWarning", () => {
  // Regra 8: `Administrator` em servidor Windows é legítimo — avisa, não bloqueia.
  test("avisa quando há letra maiúscula", () => {
    expect(usernameNeedsWarning("Root")).toBe(true);
    expect(usernameNeedsWarning("deploY")).toBe(true);
    expect(usernameNeedsWarning("Ádmin")).toBe(true);
  });

  test("não avisa em minúsculas, números ou campo vazio", () => {
    expect(usernameNeedsWarning("root")).toBe(false);
    expect(usernameNeedsWarning("deploy-01_ci")).toBe(false);
    expect(usernameNeedsWarning("")).toBe(false);
  });
});

describe("formToInput", () => {
  const form = (over: Partial<HostFormValues> = {}): HostFormValues => ({
    ...emptyHostForm(),
    alias: " vps ",
    hostname: " 203.0.113.7 ",
    username: "root",
    ...over,
  });

  const input = (values: HostFormValues) => {
    const result = formToInput(values);
    if (!result.ok) throw new Error(`esperava ok, veio ${result.errorKey}`);
    return result.input;
  };

  test("agente envia só a chave escolhida", () => {
    const got = input(
      form({ auth_method: "agent", agent_key: AGENT_KEY, identity_file: "~/.ssh/old" }),
    );
    expect(got).toMatchObject({
      alias: "vps",
      hostname: "203.0.113.7",
      auth_method: "agent",
      agent_key: AGENT_KEY,
      identity_file: null,
    });
  });

  test("arquivo envia o caminho e larga a chave do agente", () => {
    const got = input(
      form({ auth_method: "file", identity_file: " ~/.ssh/id_ed25519 ", agent_key: AGENT_KEY }),
    );
    expect(got).toMatchObject({
      auth_method: "file",
      identity_file: "~/.ssh/id_ed25519",
      agent_key: null,
    });
  });

  // Regra 4/5: senha e automático não carregam nenhum dos dois campos, mesmo
  // que o dono tenha passado por outro método antes de escolher.
  test("senha e automático não levam resto de outro método", () => {
    for (const method of ["password", "auto"] as const) {
      const got = input(
        form({ auth_method: method, identity_file: "~/.ssh/x", agent_key: AGENT_KEY }),
      );
      expect(got.auth_method).toBe(method);
      expect(got.identity_file).toBeNull();
      expect(got.agent_key).toBeNull();
    }
  });

  test("usuário com maiúscula é enviado como digitado", () => {
    expect(input(form({ username: "Administrator" })).username).toBe(
      "Administrator",
    );
  });

  // Sem isso o core responderia `ssh.agent_key_invalid` para quem só esqueceu
  // de escolher uma chave — texto certo para chave forjada, errado aqui.
  test("agente sem chave escolhida pede a escolha", () => {
    expect(formToInput(form({ auth_method: "agent" }))).toEqual({
      ok: false,
      errorKey: "hostAuthAgentKeyRequired",
      field: "form",
    });
  });

  test("arquivo sem caminho pede o caminho", () => {
    expect(
      formToInput(form({ auth_method: "file", identity_file: "  " })),
    ).toEqual({
      ok: false,
      errorKey: "hostAuthIdentityFileRequired",
      field: "form",
    });
  });

  test("apelido e host são obrigatórios", () => {
    expect(formToInput(form({ alias: " " }))).toMatchObject({
      ok: false,
      errorKey: "hostFieldAliasRequired",
    });
    expect(formToInput(form({ hostname: "" }))).toMatchObject({
      ok: false,
      errorKey: "hostFieldHostnameRequired",
    });
  });

  test("porta vazia é nula; porta fora de 1..65535 é recusada", () => {
    expect(input(form({ port: "" })).port).toBeNull();
    expect(input(form({ port: " 2222 " })).port).toBe(2222);
    for (const port of ["0", "65536", "22.5", "abc"]) {
      expect(formToInput(form({ port }))).toMatchObject({
        ok: false,
        errorKey: "hostFieldPortInvalid",
      });
    }
  });

  test("túneis: linha vazia some, dinâmico perde destino, destino é aparado", () => {
    const got = input(
      form({
        tunnels: [
          { kind: "local", listen_port: 0, listen_host: null, target_host: "localhost", target_port: null },
          { kind: "local", listen_port: 5433, listen_host: null, target_host: " localhost ", target_port: 5432 },
          { kind: "dynamic", listen_port: 1080, listen_host: null, target_host: "x", target_port: 9 },
        ],
      }),
    );
    expect(got.tunnels).toEqual([
      { kind: "local", listen_port: 5433, listen_host: null, target_host: "localhost", target_port: 5432 },
      { kind: "dynamic", listen_port: 1080, listen_host: null, target_host: null, target_port: null },
    ]);
  });

  test("túnel incompleto é recusado no campo de túneis", () => {
    expect(
      formToInput(
        form({
          tunnels: [
            { kind: "local", listen_port: 5433, listen_host: null, target_host: "", target_port: 5432 },
          ],
        }),
      ),
    ).toEqual({ ok: false, errorKey: "hostTunnelInvalid", field: "tunnels" });
  });
});

describe("applyInputToHost", () => {
  test("edição troca os campos do formulário e preserva identidade e datas", () => {
    const before = host({
      id: "h9",
      position: 3,
      identity_file: "~/.ssh/legacy",
      last_connected_at: "2026-09-15T10:00:00Z",
    });
    const result = formToInput({
      ...hostToForm(before),
      username: "deploy",
      auth_method: "agent",
      agent_key: AGENT_KEY,
    });
    if (!result.ok) throw new Error(result.errorKey);
    const after = applyInputToHost(before, result.input);
    expect(after).toMatchObject({
      id: "h9",
      position: 3,
      created_at: before.created_at,
      last_connected_at: "2026-09-15T10:00:00Z",
      username: "deploy",
      auth_method: "agent",
      agent_key: AGENT_KEY,
      identity_file: null,
    });
  });
});

describe("lista de chaves do agente", () => {
  test("o tipo é o primeiro campo da chave pública", () => {
    expect(agentKeyType(AGENT_KEY.public_key)).toBe("ssh-ed25519");
    expect(agentKeyType("  ")).toBe("");
  });

  // Agente inalcançável e agente sem chaves pedem textos diferentes: um manda
  // configurar o agente, o outro manda adicionar a chave nele.
  test("sem socket, vazio e com chaves são estados distintos", () => {
    expect(agentListingState({ socket: null, keys: [] })).toBe("no_agent");
    expect(agentListingState({ socket: "/tmp/agent.sock", keys: [] })).toBe("empty");
    expect(
      agentListingState({ socket: "/tmp/agent.sock", keys: [AGENT_KEY] }),
    ).toBe("keys");
  });
});

describe("todo erro do formulário tem texto em pt-BR e en", () => {
  const source = readFileSync(new URL("../i18n/index.ts", import.meta.url), "utf8");
  const occurrences = (key: string) =>
    source.split(new RegExp(`\\n\\s+"?${key}"?:`)).length - 1;

  test("chaves devolvidas por formToInput", () => {
    const base = { ...emptyHostForm(), alias: "a", hostname: "h" };
    const tunnel = { kind: "local" as const, listen_port: 1, listen_host: null, target_host: "", target_port: 1 };
    const failures = [
      formToInput({ ...base, alias: "" }),
      formToInput({ ...base, hostname: "" }),
      formToInput({ ...base, port: "0" }),
      formToInput({ ...base, tunnels: [tunnel] }),
      formToInput({ ...base, auth_method: "agent" }),
      formToInput({ ...base, auth_method: "file" }),
    ];
    const keys = failures.map((r) => (r.ok ? "ok" : r.errorKey));
    expect(keys.filter((k) => occurrences(k) !== 2)).toEqual([]);
  });
});

describe("evento de lista de hosts alterada", () => {
  // O nome é contrato entre duas linguagens: divergir não quebra build nenhum,
  // só deixa a tela escutando um evento que nunca chega.
  test("tem o mesmo nome que o core emite", () => {
    const core = readFileSync(
      new URL("../../src-tauri/src/lib.rs", import.meta.url),
      "utf8",
    );
    const emitted = core.match(/const EVENT_HOSTS_CHANGED: &str = "([^"]+)";/);
    expect(emitted?.[1]).toBe(EVENT_HOSTS_CHANGED);
  });
});

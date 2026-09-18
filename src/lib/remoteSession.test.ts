import { describe, expect, it } from "bun:test";

import {
  integrationNotice,
  isIntegratedSession,
  nextTransport,
  silenceOscReply,
  silenceReply,
  paneNotice,
  remoteAgentNotice,
  sidebarChips,
  toolbarChips,
} from "./remoteSession";

describe("isIntegratedSession", () => {
  it("shell local é sessão integrada", () => {
    expect(
      isIntegratedSession({ kind: { type: "shell" }, integration: undefined }),
    ).toBe(true);
  });

  it("SSH Session que o core disse ser integrada também é", () => {
    expect(
      isIntegratedSession({
        kind: { type: "ssh", host_id: "h1" },
        integration: { state: "integrated", reason: "ok" },
      }),
    ).toBe(true);
  });

  it("SSH Session comum continua fora — é o terminal cru de hoje", () => {
    expect(
      isIntegratedSession({
        kind: { type: "ssh", host_id: "h1" },
        integration: { state: "plain", reason: "unsupported-shell" },
      }),
    ).toBe(false);
  });

  it("sem resposta do core ainda, não é integrada", () => {
    // O intervalo entre abrir a sessão e o evento chegar. Assumir "sim" aqui
    // poria a linha do TYBA na frente de um terminal cru.
    expect(
      isIntegratedSession({
        kind: { type: "ssh", host_id: "h1" },
        integration: undefined,
      }),
    ).toBe(false);
  });

  it("sessão de agente nunca é integrada", () => {
    expect(
      isIntegratedSession({
        kind: { type: "agent", runner: "claude_code" },
        integration: undefined,
      }),
    ).toBe(false);
  });
});

describe("integrationNotice", () => {
  it("a chave desligada do Host é explicada no pane", () => {
    expect(
      integrationNotice({ state: "plain", reason: "host-switch-off" }),
    ).toEqual({ messageKey: "sshIntegrationOffSwitch", params: {} });
  });
});

describe("integrationNotice — shell recusado", () => {
  it("nomeia o shell que o servidor usa", () => {
    // Sem o nome a linha não explica nada: "shell não suportado" não diz ao
    // dono o que ele tem de trocar.
    expect(
      integrationNotice({
        state: "plain",
        reason: "unsupported-shell",
        detail: "fish",
      }),
    ).toEqual({
      messageKey: "sshIntegrationUnsupportedShell",
      params: { shell: "fish" },
    });
  });
});

describe("integrationNotice — os outros motivos", () => {
  it("shell recusado sem nome não deixa buraco na frase", () => {
    expect(
      integrationNotice({ state: "plain", reason: "unsupported-shell" }),
    ).toEqual({ messageKey: "sshIntegrationUnsupportedShellUnnamed", params: {} });
  });
});

describe("integrationNotice — detecção e sessão antiga", () => {
  it("não ter conseguido detectar o shell tem linha própria", () => {
    expect(integrationNotice({ state: "plain", reason: "undetected" })).toEqual({
      messageKey: "sshIntegrationUndetected",
      params: {},
    });
  });
});

describe("integrationNotice — sessão de antes da versão (regra 12)", () => {
  it("explica que a sessão é anterior à integração", () => {
    expect(integrationNotice({ state: "plain", reason: "from-before" })).toEqual({
      messageKey: "sshIntegrationFromBefore",
      params: {},
    });
  });
});

describe("integrationNotice — motivo que a tela não conhece", () => {
  it("diz que a sessão é comum, sem inventar a causa", () => {
    // Um motivo novo no core (a versão seguinte pode ter um) não pode deixar o
    // pane mudo: a sessão É comum, e isso já é o que o dono precisa ver.
    expect(
      integrationNotice({ state: "plain", reason: "algo-que-virá" }),
    ).toEqual({ messageKey: "sshIntegrationPlain", params: {} });
  });
});

describe("integrationNotice — integrada sem persistência (regra 13)", () => {
  it("host sem tmux ganha a ressalva, sem deixar de ser integrada", () => {
    expect(
      integrationNotice({
        state: "integrated",
        reason: "ok",
        persistence: "ephemeral",
      }),
    ).toEqual({ messageKey: "sshIntegrationEphemeral", params: {} });
  });

  it("sem resposta do core, a tela não afirma nada sobre persistência", () => {
    // `unknown` é o core que não conseguiu perguntar (sem canal, Host de senha
    // sem master). Uma ressalva aqui seria inventar um fato sobre o servidor.
    expect(
      integrationNotice({
        state: "integrated",
        reason: "ok",
        persistence: "unknown",
      }),
    ).toBeNull();
  });

  it("host com tmux não ganha ressalva nenhuma", () => {
    expect(
      integrationNotice({
        state: "integrated",
        reason: "ok",
        persistence: "persistent",
      }),
    ).toBeNull();
  });

  it("sessão comum continua explicando por que é comum", () => {
    // Um servidor sem tmux também pode recusar o shell. Ali a linha que importa
    // é a de hoje: a sessão nem integrada é, e falar de persistência antes
    // disso responderia a pergunta errada.
    expect(
      integrationNotice({
        state: "plain",
        reason: "unsupported-shell",
        detail: "fish",
        persistence: "ephemeral",
      }),
    ).toEqual({
      messageKey: "sshIntegrationUnsupportedShell",
      params: { shell: "fish" },
    });
  });
});

describe("integrationNotice — sessão integrada", () => {
  it("não ganha faixa nenhuma", () => {
    expect(integrationNotice({ state: "integrated", reason: "ok" })).toBeNull();
  });

  it("nem enquanto a resposta do core não chegou", () => {
    expect(integrationNotice(undefined)).toBeNull();
  });
});

const localChips = {
  cwd: "/Users/dono/projeto",
  branch: { state: "known", label: "main", sessionId: "s1" } as const,
  snapshot: undefined,
};

describe("toolbarChips", () => {
  it("sessão local continua lendo a máquina local", () => {
    expect(
      toolbarChips({ remote: false, chips: undefined, local: localChips }),
    ).toEqual({
      cwd: "/Users/dono/projeto",
      branch: localChips.branch,
      snapshot: undefined,
      remoteChanged: null,
    });
  });
});

describe("toolbarChips — sessão SSH", () => {
  it("mostra pasta, branch e contagem do servidor", () => {
    expect(
      toolbarChips({
        remote: true,
        chips: {
          cwd: "/srv/app",
          git: { branch: "deploy", changed: 3 },
        },
        local: localChips,
      }),
    ).toEqual({
      cwd: "/srv/app",
      branch: { state: "known", label: "deploy", sessionId: null },
      snapshot: undefined,
      remoteChanged: 3,
    });
  });
});

describe("toolbarChips — SSH sem canal (regra 25)", () => {
  it("os chips somem em vez de mostrar valor local", () => {
    // Sem ControlMaster o core responde `null`. Mostrar a pasta e a branch
    // daqui seria afirmar sobre o servidor o que é da máquina local.
    expect(
      toolbarChips({ remote: true, chips: null, local: localChips }),
    ).toEqual({
      cwd: null,
      branch: null,
      snapshot: undefined,
      remoteChanged: null,
    });
  });

  it("e também enquanto a primeira resposta não chegou", () => {
    expect(
      toolbarChips({ remote: true, chips: undefined, local: localChips }),
    ).toEqual({
      cwd: null,
      branch: null,
      snapshot: undefined,
      remoteChanged: null,
    });
  });
});

describe("toolbarChips — servidor limpo", () => {
  it("não mostra o chip de diff com zero mudanças", () => {
    // Mesma regra do chip local, que só aparece com o repositório sujo.
    const chips = toolbarChips({
      remote: true,
      chips: { cwd: "/srv/app", git: { branch: "main", changed: 0 } },
      local: localChips,
    });
    expect(chips.remoteChanged).toBeNull();
    expect(chips.branch).toEqual({
      state: "known",
      label: "main",
      sessionId: null,
    });
  });
});

const localSidebar = {
  branch: "main",
  status: { dirty: true, changed: 106, insertions: 129197, deletions: 0 },
};

describe("sidebarChips — SSH antes do servidor responder (regra 23)", () => {
  it("não mostra branch nem diff da máquina local", () => {
    // A janela de reconexão: o `ssh` local ainda tem o cwd de onde o app subiu,
    // então o snapshot local resolve — e é do repositório de CÁ.
    expect(
      sidebarChips({
        remote: true,
        showStatus: true,
        chips: undefined,
        local: localSidebar,
      }),
    ).toEqual({ branch: null, status: undefined, remoteChanged: null });
  });
});

describe("sidebarChips — preferência 'status do git na sidebar' desligada", () => {
  it("esconde também o diff do servidor, e mantém a branch", () => {
    // A preferência fala da LINHA, não da fonte: o chip de diff sai dos dois
    // lados, e a branch fica dos dois lados, como já era no caso local.
    expect(
      sidebarChips({
        remote: true,
        showStatus: false,
        chips: { cwd: "/srv/app", git: { branch: "deploy", changed: 3 } },
        local: localSidebar,
      }),
    ).toEqual({ branch: "deploy", status: undefined, remoteChanged: null });
  });

  it("no caso local, some o diff e fica a branch", () => {
    expect(
      sidebarChips({
        remote: false,
        showStatus: false,
        chips: undefined,
        local: localSidebar,
      }),
    ).toEqual({ branch: "main", status: undefined, remoteChanged: null });
  });
});

describe("sidebarChips — SSH com a resposta do servidor", () => {
  it("mostra a branch e a contagem do servidor", () => {
    expect(
      sidebarChips({
        remote: true,
        showStatus: true,
        chips: { cwd: "/srv/app", git: { branch: "deploy", changed: 3 } },
        local: localSidebar,
      }),
    ).toEqual({ branch: "deploy", status: undefined, remoteChanged: 3 });
  });

  it("servidor limpo não ganha chip de diff", () => {
    // Mesma regra do chip local, que só aparece com o repositório sujo.
    expect(
      sidebarChips({
        remote: true,
        showStatus: true,
        chips: { cwd: "/srv/app", git: { branch: "main", changed: 0 } },
        local: localSidebar,
      }),
    ).toEqual({ branch: "main", status: undefined, remoteChanged: null });
  });

  it("sem canal (regra 25) a linha fica sem branch e sem diff", () => {
    expect(
      sidebarChips({
        remote: true,
        showStatus: true,
        chips: null,
        local: localSidebar,
      }),
    ).toEqual({ branch: null, status: undefined, remoteChanged: null });
  });
});

describe("sidebarChips — sessão local", () => {
  it("continua mostrando a branch e o diff do repositório dela", () => {
    expect(
      sidebarChips({
        remote: false,
        showStatus: true,
        chips: undefined,
        local: localSidebar,
      }),
    ).toEqual({
      branch: "main",
      status: localSidebar.status,
      remoteChanged: null,
    });
  });

  it("fora de repositório não mostra nada", () => {
    // `RepoSnapshot` devolve `null` no lugar de `undefined` (é o que vem do
    // core), e a linha não pode distinguir os dois.
    expect(
      sidebarChips({
        remote: false,
        showStatus: true,
        chips: undefined,
        local: { branch: null, status: null },
      }),
    ).toEqual({ branch: null, status: undefined, remoteChanged: null });
  });
});

describe("remoteAgentNotice (regra 26)", () => {
  it("nomeia o agente que está rodando no servidor", () => {
    expect(
      remoteAgentNotice({
        command: {
          command: "claude --dangerously-skip-permissions",
          running: true,
          agent_match: true,
          continuation: false,
        },
        confirmed: "claude --dangerously-skip-permissions",
      }),
    ).toEqual({ binary: "claude" });
  });
});

describe("remoteAgentNotice — quando a faixa sai da tela", () => {
  const claude = {
    command: "claude",
    running: true,
    agent_match: true,
    continuation: false,
  };

  it("some quando o comando termina", () => {
    expect(
      remoteAgentNotice({
        command: { ...claude, running: false },
        confirmed: "claude",
      }),
    ).toBeNull();
  });

  it("a resposta de um comando não pinta a faixa sobre o seguinte", () => {
    // O core responde em outra volta do loop: sem comparar o comando, um `ls`
    // logo depois do agente herdaria a faixa.
    expect(
      remoteAgentNotice({
        command: { ...claude, command: "ls" },
        confirmed: "claude",
      }),
    ).toBeNull();
  });

  it("sem resposta do core não há faixa", () => {
    expect(remoteAgentNotice({ command: claude, confirmed: null })).toBeNull();
  });
});

describe("paneNotice", () => {
  const plain = { state: "plain", reason: "from-before" } as const;

  it("o agente sem jaula ganha da linha de integração", () => {
    // As duas faixas moram no mesmo lugar do pane. Enquanto um agente roda no
    // servidor sem jaula e sem inbox, é isso que o dono precisa ler.
    expect(
      paneNotice({ integration: plain, agent: { binary: "claude" } }),
    ).toEqual({
      messageKey: "sshRemoteAgentNotice",
      params: { binary: "claude" },
      tone: "amber",
    });
  });

  it("sem agente, explica por que a sessão é comum", () => {
    expect(paneNotice({ integration: plain, agent: null })).toEqual({
      messageKey: "sshIntegrationFromBefore",
      params: {},
      tone: "cyan",
    });
  });

  const efemera = {
    state: "integrated",
    reason: "ok",
    persistence: "ephemeral",
  } as const;

  it("a sessão integrada sem persistência ganha a faixa informativa", () => {
    expect(paneNotice({ integration: efemera, agent: null })).toEqual({
      messageKey: "sshIntegrationEphemeral",
      params: {},
      tone: "cyan",
    });
  });

  it("e o agente sem jaula continua na frente dela", () => {
    expect(
      paneNotice({ integration: efemera, agent: { binary: "claude" } }),
    ).toEqual({
      messageKey: "sshRemoteAgentNotice",
      params: { binary: "claude" },
      tone: "amber",
    });
  });

  it("sessão integrada e sem agente não tem faixa", () => {
    expect(
      paneNotice({
        integration: { state: "integrated", reason: "ok" },
        agent: null,
      }),
    ).toBeNull();
  });
});

describe("nextTransport", () => {
  it("o anúncio do modo de controle troca o transporte da sessão", () => {
    expect(nextTransport("raw", "tmux_control")).toBe("tmux_control");
  });

  it("transporte que esta versão não conhece mantém o estado atual", () => {
    expect(nextTransport("raw", "mosh")).toBe("raw");
  });

  it("voltar a `raw` depois do controle é anúncio válido, não engano", () => {
    // Um respawn no mesmo id (reconexão do Cano) reanuncia `raw` no spawn, e a
    // sessão pode renascer comum — integração desligada, host sem tmux. Uma
    // máquina só-de-ida calaria o xterm para sempre nesse caso.
    expect(nextTransport("tmux_control", "raw")).toBe("raw");
  });
});

describe("silenceReply", () => {
  it("num transporte de controle o xterm não responde DA1", () => {
    expect(
      silenceReply({
        transport: "tmux_control",
        id: { final: "c" },
        params: [0],
      }),
    ).toBe(true);
  });

  it("numa sessão `raw` nada muda: quem responde continua sendo o xterm", () => {
    expect(
      silenceReply({ transport: "raw", id: { final: "c" }, params: [0] }),
    ).toBe(false);
  });

  it("`n` sem parâmetro de relatório não é consulta: o default roda", () => {
    // Final ambíguo: só DSR 5 e CPR 6 têm resposta no xterm.js. Calar o `n`
    // inteiro tiraria do default sequências que ele trata sem responder.
    expect(
      silenceReply({
        transport: "tmux_control",
        id: { final: "n" },
        params: [0],
      }),
    ).toBe(false);
  });

  it("DA2 também é calada — foi a metade `0;276;0c` do lixo na tela", () => {
    expect(
      silenceReply({
        transport: "tmux_control",
        id: { prefix: ">", final: "c" },
        params: [0],
      }),
    ).toBe(true);
  });

  it("DECXCPR é calada; o resto do DSR privado fica com o default", () => {
    const privateDsr = (param: number) =>
      silenceReply({
        transport: "tmux_control",
        id: { prefix: "?", final: "n" },
        params: [param],
      });
    expect(privateDsr(6)).toBe(true);
    // `?15n` (impressora) e `?25n` (teclas de usuário) o xterm.js reconhece e
    // NÃO responde — calar não mudaria a tela e esconderia a intenção.
    expect(privateDsr(15)).toBe(false);
  });

  it("DECRQM é calada nos dois sabores — ela SEMPRE responde", () => {
    const requestMode = (id: { prefix?: string; intermediates: string }) =>
      silenceReply({
        transport: "tmux_control",
        id: { ...id, final: "p" },
        params: [2004],
      });
    expect(requestMode({ intermediates: "$" })).toBe(true);
    expect(requestMode({ prefix: "?", intermediates: "$" })).toBe(true);
  });

  it("do XTWINOPS só os relatórios são calados", () => {
    const winop = (...params: number[]) =>
      silenceReply({
        transport: "tmux_control",
        id: { final: "t" },
        params,
      });
    expect(winop(18)).toBe(true); // tamanho da área em células
    expect(winop(14)).toBe(true); // área em pixels
    expect(winop(16)).toBe(true); // célula em pixels
    // `14;2t` pede o tamanho da JANELA, que o xterm.js não relata; e 22/23 são
    // empilhar e desempilhar título — calar apagaria o título do pane.
    expect(winop(14, 2)).toBe(false);
    expect(winop(22, 0)).toBe(false);
    expect(winop(23, 0)).toBe(false);
  });

  it("DECRQSS, que é DCS, é calada pelo mesmo caminho", () => {
    expect(
      silenceReply({
        transport: "tmux_control",
        kind: "dcs",
        id: { intermediates: "$", final: "q" },
        params: [],
      }),
    ).toBe(true);
  });

  it("a mesma forma vinda como CSI não é a DECRQSS e fica com o default", () => {
    // CSI e DCS são espaços de nome separados no xterm.js: `CSI " q`
    // (`selectProtected`) partilha o final `q` e não responde nada.
    expect(
      silenceReply({
        transport: "tmux_control",
        id: { intermediates: "$", final: "q" },
        params: [],
      }),
    ).toBe(false);
  });

  it("DA3 vai junto, por precaução", () => {
    // O xterm.js 5.5.0 não registra `=c` (conferido no sourcemap): hoje calar
    // não muda nada. Está aqui porque o dia em que ele responder não pode ser
    // descoberto pelo lixo no prompt do servidor.
    expect(
      silenceReply({
        transport: "tmux_control",
        id: { prefix: "=", final: "c" },
        params: [0],
      }),
    ).toBe(true);
  });
});

describe("silenceOscReply", () => {
  it("a consulta de cor de fundo é calada no transporte de controle", () => {
    expect(
      silenceOscReply({ transport: "tmux_control", ident: 11, data: "?" }),
    ).toBe(true);
  });

  it("pintar uma cor não é consulta: o default pinta", () => {
    expect(
      silenceOscReply({
        transport: "tmux_control",
        ident: 11,
        data: "#1e1e1e",
      }),
    ).toBe(false);
  });

  it("na OSC 4 só o slot da COR pode ser consulta, nunca o do índice", () => {
    const indexed = (data: string) =>
      silenceOscReply({ transport: "tmux_control", ident: 4, data });
    expect(indexed("1;?")).toBe(true);
    // `?` no lugar do índice não é consulta nenhuma: o xterm.js exige dígitos
    // ali e descarta o par. Calar aqui seria calar o que ninguém perguntou.
    expect(indexed("?;#ff0000")).toBe(false);
  });

  it("consulta misturada com pintura cala as duas — de propósito", () => {
    // Armadilha: o handler é tudo ou nada, então a cor do meio não é pintada.
    // Perder uma cor é cosmético; devolver a resposta é digitação no pane.
    expect(
      silenceOscReply({
        transport: "tmux_control",
        ident: 4,
        data: "1;#ff0000;2;?",
      }),
    ).toBe(true);
  });

  it("sessão `raw` responde a cor como sempre respondeu", () => {
    expect(silenceOscReply({ transport: "raw", ident: 11, data: "?" })).toBe(
      false,
    );
  });
});

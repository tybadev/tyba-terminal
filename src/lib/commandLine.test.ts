import { describe, expect, it } from "bun:test";

import {
  enterAplicaSugestao,
  commandPrefix,
  boxAcceptsTyping,
  boxIsMounted,
  caretLine,
  clearsDraft,
  controlBytes,
  ghostDoToken,
  ghostFor,
  isArrowKey,
  keyboardOwner,
  lineState,
  lineVisible,
  paneLine,
  lineToken,
  menuItems,
  discoveryItems,
  argValue,
  moveDiscovery,
  menuRows,
  moveSelection,
  pathToken,
  programName,
  promptModeEnabled,
  replaceToken,
  swallowsArrow,
  type LineState,
  type OwnerInput,
} from "./commandLine";

const atPrompt: OwnerInput = {
  promptMode: true,
  kind: { type: "shell" },
  altScreen: false,
  command: { command: null, running: false, agent_match: false, continuation: false },
  integrated: true,
};

describe("keyboardOwner", () => {
  it("dá a linha ao TYBA quando o shell está no prompt", () => {
    expect(keyboardOwner(atPrompt)).toBe("tybaLine");
  });

  it("devolve o teclado ao terminal enquanto um comando roda", () => {
    // A regra que impede o usuário de digitar a senha do sudo numa caixa que
    // não vai a lugar nenhum.
    const running = {
      ...atPrompt,
      command: { command: "ssh prod", running: true, agent_match: false, continuation: false },
    };
    expect(keyboardOwner(running)).toBe("terminal");
  });

  it("devolve o teclado ao terminal em alt-screen", () => {
    expect(keyboardOwner({ ...atPrompt, altScreen: true })).toBe("terminal");
  });

  it("alt-screen vence mesmo com o shell ocioso", () => {
    // `vim` deixa o shell sem comando rodando do ponto de vista do OSC 133,
    // mas a tela é dele.
    const inVim = { ...atPrompt, altScreen: true, command: undefined };
    expect(keyboardOwner(inVim)).toBe("terminal");
  });

  it("nunca assume o prompt sem shell integration", () => {
    expect(keyboardOwner({ ...atPrompt, integrated: false })).toBe("terminal");
  });

  it("não toma a linha de sessão de agente", () => {
    const agent: OwnerInput = {
      ...atPrompt,
      kind: { type: "agent", runner: "claude_code" },
    };
    expect(keyboardOwner(agent)).toBe("terminal");
  });

  // Regra 15 da spec `ssh-sessao-integrada`: a linha do TYBA passa a valer
  // para a SSH Session INTEGRADA. O critério anterior ("nunca em sessão ssh")
  // valia enquanto o shell remoto era cru; com o rc do TYBA do outro lado, o
  // `133;A` que autoriza a linha vem do servidor.
  it("toma a linha da sessão ssh integrada", () => {
    const ssh: OwnerInput = {
      ...atPrompt,
      kind: { type: "ssh", host_id: "h1" },
      sessionIntegration: { state: "integrated", reason: "ok" },
    };
    expect(keyboardOwner(ssh)).toBe("tybaLine");
  });

  it("não toma a linha de sessão ssh comum", () => {
    const ssh: OwnerInput = {
      ...atPrompt,
      kind: { type: "ssh", host_id: "h1" },
      sessionIntegration: { state: "plain", reason: "unsupported-shell" },
    };
    expect(keyboardOwner(ssh)).toBe("terminal");
  });

  it("nem enquanto o core não disse se a sessão ssh é integrada", () => {
    const ssh: OwnerInput = {
      ...atPrompt,
      kind: { type: "ssh", host_id: "h1" },
    };
    expect(keyboardOwner(ssh)).toBe("terminal");
  });

  it("na sessão ssh integrada as três situações valem igual", () => {
    // Critério de aceite: a linha aparece na sessão integrada E respeita os
    // estados de teclado. O `sudo` que pede senha do outro lado do ssh lê
    // stdin do mesmo jeito que o local, e o `PS2` remoto é continuado pela
    // linha como o local.
    const remote: OwnerInput = {
      ...atPrompt,
      kind: { type: "ssh", host_id: "h1" },
      sessionIntegration: { state: "integrated", reason: "ok" },
    };
    expect(keyboardOwner({ ...remote, altScreen: true })).toBe("terminal");
    expect(
      keyboardOwner({
        ...remote,
        command: {
          command: "sudo apt update",
          running: true,
          agent_match: false,
          continuation: false,
        },
      }),
    ).toBe("terminal");
    expect(
      keyboardOwner({
        ...remote,
        command: {
          command: "for i in 1 2 3",
          running: false,
          agent_match: false,
          continuation: true,
        },
      }),
    ).toBe("tybaLine");
  });

  it("a rajada do broadcast fica com o terminal", () => {
    // Regra 17: o broadcast é interceptado no xterm (`onData`). Com a linha do
    // TYBA dona do teclado o xterm vira somente-leitura, e a rajada não sairia
    // de lugar nenhum — a sessão integrada seria a única onde o broadcast
    // deixou de funcionar.
    const remote: OwnerInput = {
      ...atPrompt,
      kind: { type: "ssh", host_id: "h1" },
      sessionIntegration: { state: "integrated", reason: "ok" },
      broadcasting: true,
    };
    expect(keyboardOwner(remote)).toBe("terminal");
  });

  it("respeita a válvula de escape do usuário", () => {
    expect(keyboardOwner({ ...atPrompt, promptMode: false })).toBe("terminal");
  });
});

describe("lineState", () => {
  it("é editável só quando o shell está no prompt", () => {
    expect(lineState(atPrompt)).toBe("own");
  });

  it("diz por que não é editável, em vez de sumir", () => {
    // A caixa sumir e voltar a cada comando redimensionava o terminal duas
    // vezes por execução.
    expect(
      lineState({
        ...atPrompt,
        command: { command: "sleep 5", running: true, agent_match: false, continuation: false },
      }),
    ).toBe("running");
    expect(lineState({ ...atPrompt, altScreen: true })).toBe("app");
    expect(lineState({ ...atPrompt, promptMode: false })).toBe("waiting");
  });

  it("desligado continua na tela, dizendo que está desligado", () => {
    // A linha sumir sem explicação foi o que fez um ⌘⇧L a mais parecer defeito.
    expect(
      lineState({ ...atPrompt, promptMode: false, reported: false }),
    ).toBe("off");
  });

  it("app de tela cheia vence comando rodando", () => {
    expect(
      lineState({
        ...atPrompt,
        altScreen: true,
        command: { command: "vim", running: true, agent_match: false, continuation: false },
      }),
    ).toBe("app");
  });
});

describe("ghostFor", () => {
  const hits = [
    { command: "cargo test --lib" },
    { command: "cargo clippy" },
    { command: "git status" },
  ];

  it("completa com a primeira sugestão que é prefixo", () => {
    expect(ghostFor("cargo t", hits)).toBe("est --lib");
  });

  it("pula sugestão que não começa com o digitado", () => {
    // "git status" está na lista, mas o ghost tem de casar o prefixo — senão o
    // cinza mentiria sobre o que a seta vai completar.
    expect(ghostFor("gi", hits)).toBe("t status");
    expect(ghostFor("zzz", hits)).toBe("");
  });

  it("não sugere nada com a linha vazia ou só espaço", () => {
    expect(ghostFor("", hits)).toBe("");
    expect(ghostFor("   ", hits)).toBe("");
  });

  it("não sugere quando o digitado já é o comando inteiro", () => {
    expect(ghostFor("cargo clippy", hits)).toBe("");
  });
});

describe("pathToken", () => {
  const at = (text: string) => pathToken(text, text.length);

  it("completa argumento como caminho", () => {
    expect(at("cd tyba")).toEqual({ start: 3, value: "tyba" });
    expect(at("cat src/lib/ip")).toEqual({ start: 4, value: "src/lib/ip" });
  });

  it("não completa a primeira palavra: ali é posição de comando", () => {
    // Num diretório com `teste/`, completar `te` para `teste/` quebraria quem
    // está digitando `test`.
    expect(at("te")).toBeNull();
    expect(at("git")).toBeNull();
  });

  it("completa a primeira palavra quando ela já se declara caminho", () => {
    expect(at("./scr")).toEqual({ start: 0, value: "./scr" });
    expect(at("../ou")).toEqual({ start: 0, value: "../ou" });
    expect(at("~/proj")).toEqual({ start: 0, value: "~/proj" });
    expect(at("/usr/lo")).toEqual({ start: 0, value: "/usr/lo" });
    expect(at("bin/tool")).toEqual({ start: 0, value: "bin/tool" });
  });

  it("não completa depois de espaço nem em linha vazia", () => {
    expect(at("cd ")).toBeNull();
    expect(at("")).toBeNull();
  });

  it("usa o token sob o cursor, não o fim da linha", () => {
    expect(pathToken("cp src dst", 5)).toEqual({ start: 3, value: "sr" });
  });
});

describe("replaceToken", () => {
  it("troca só o token e devolve o cursor depois dele", () => {
    const text = "cp src dst";
    const token = pathToken(text, 5)!;
    expect(replaceToken(text, token, "src/")).toEqual({
      text: "cp src/c dst",
      caret: 7,
    });
  });
});

describe("lineToken", () => {
  const at = (text: string) => lineToken(text, text.length);

  it("dá o token e o contexto que vem antes dele", () => {
    expect(at("git co")).toEqual({ start: 4, value: "co", prefix: "git " });
    expect(at("cargo test --l")).toEqual({
      start: 11,
      value: "--l",
      prefix: "cargo test ",
    });
  });

  it("aceita token vazio logo após o espaço", () => {
    // `git ` sem nada digitado ainda deve oferecer os subcomandos usados.
    expect(at("git ")).toEqual({ start: 4, value: "", prefix: "git " });
  });

  it("não completa a primeira palavra: ali não há contexto", () => {
    expect(at("gi")).toBeNull();
    expect(at("")).toBeNull();
  });
});

describe("controlBytes", () => {
  it("traduz os sinais que a caixa nunca consome", () => {
    expect(controlBytes({ key: "c", ctrl: true, meta: false, alt: false })).toBe(
      "\x03",
    );
    expect(controlBytes({ key: "d", ctrl: true, meta: false, alt: false })).toBe(
      "\x04",
    );
    expect(controlBytes({ key: "z", ctrl: true, meta: false, alt: false })).toBe(
      "\x1a",
    );
  });

  it("ignora letra sem ctrl e chord com meta", () => {
    expect(controlBytes({ key: "c", ctrl: false, meta: false, alt: false })).toBeNull();
    expect(controlBytes({ key: "c", ctrl: true, meta: true, alt: false })).toBeNull();
  });

  it("ignora ctrl em tecla que não é sinal", () => {
    expect(controlBytes({ key: "a", ctrl: true, meta: false, alt: false })).toBeNull();
  });

  it("só o Ctrl+C limpa o rascunho", () => {
    expect(clearsDraft({ key: "c", ctrl: true, meta: false, alt: false })).toBe(true);
    expect(clearsDraft({ key: "d", ctrl: true, meta: false, alt: false })).toBe(false);
  });
});

describe("swallowsArrow", () => {
  const base = { running: true, lineEcho: true, altScreen: false };

  it("engole a seta com o tty em modo linha — ela só viraria `^[[A` gravado", () => {
    expect(swallowsArrow(base)).toBe(true);
  });

  it("deixa passar em raw: é o menu do `npm create` lendo tecla a tecla", () => {
    expect(swallowsArrow({ ...base, lineEcho: false })).toBe(false);
  });

  it("deixa passar em alt-screen — a tela é do vim e as setas também", () => {
    expect(swallowsArrow({ ...base, altScreen: true })).toBe(false);
  });

  it("sem comando rodando não é com ela: a linha do TYBA já é dona do teclado", () => {
    expect(swallowsArrow({ ...base, running: false })).toBe(false);
    expect(swallowsArrow({ ...base, running: false, lineEcho: false })).toBe(
      false,
    );
  });

  it("o mesmo comando troca de modo no meio, e a decisão acompanha", () => {
    // `npm create`: canônico no `Ok to proceed? (y)`, raw quando abre o menu.
    expect(swallowsArrow({ ...base, lineEcho: true })).toBe(true);
    expect(swallowsArrow({ ...base, lineEcho: false })).toBe(false);
  });
});

describe("isArrowKey", () => {
  it("reconhece as quatro e mais nada", () => {
    for (const k of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]) {
      expect(isArrowKey(k)).toBe(true);
    }
    // O `y` do `Ok to proceed?` também é canônico com eco: se entrasse aqui,
    // não haveria como responder ao prompt.
    expect(isArrowKey("y")).toBe(false);
    expect(isArrowKey("Enter")).toBe(false);
    expect(isArrowKey("Home")).toBe(false);
  });
});

describe("programName", () => {
  it("comando simples", () => {
    expect(programName("nvim")).toBe("nvim");
    expect(programName("htop -d 5")).toBe("htop");
  });

  it("caminho absoluto vira só o nome", () => {
    expect(programName("/usr/local/bin/nvim arquivo.ts")).toBe("nvim");
  });

  it("atribuição de ambiente na frente não é o programa", () => {
    expect(programName("EDITOR=vim FOO=bar nvim")).toBe("nvim");
  });

  it("wrapper não rouba o nome do programa", () => {
    expect(programName("sudo vim /etc/hosts")).toBe("vim");
    expect(programName("env -i CC=clang make")).toBe("make");
    expect(programName("nohup htop")).toBe("htop");
  });

  it("wrapper sozinho continua sendo o programa", () => {
    // `sudo` sem nada depois é o que o usuário digitou, e é o que ele vê.
    expect(programName("sudo")).toBe("sudo");
  });

  it("vazio e nulo não inventam nome", () => {
    expect(programName("")).toBeNull();
    expect(programName(null)).toBeNull();
    expect(programName("   ")).toBeNull();
  });

  it("o comando digitado ganha do processo em primeiro plano", () => {
    // `git log` abre o `less`; o rótulo diz "git", que é o que foi pedido.
    expect(programName("git log --oneline")).toBe("git");
  });

  it("o valor de uma flag do wrapper não é o programa", () => {
    // `sudo -u app git push`: o `app` é o usuário, não o programa. Dizer
    // "app está no controle" é errado com confiança — a pior forma de errar
    // num rótulo, porque não se parece com defeito.
    expect(programName("sudo -u app git push")).toBe("git");
    expect(programName("sudo --user app git push")).toBe("git");
    expect(programName("sudo -g wheel htop")).toBe("htop");
    expect(programName("doas -u deploy nvim /etc/hosts")).toBe("nvim");
    expect(programName("env -u LANG nvim")).toBe("nvim");
  });

  it("flag com `=` não consome o próximo token", () => {
    expect(programName("sudo --user=app git push")).toBe("git");
  });

  it("flag booleana conhecida não come o programa", () => {
    expect(programName("sudo -k make")).toBe("make");
    expect(programName("sudo -E -H make install")).toBe("make");
  });

  it("`--` encerra as flags do wrapper", () => {
    expect(programName("sudo -u app -- git push")).toBe("git");
  });

  it("flag desconhecida devolve o wrapper, não um chute", () => {
    // Sem saber se a flag consome o próximo token, qualquer resposta seria
    // adivinhação. "sudo" é impreciso e verdadeiro; "config" seria mentira.
    expect(programName("sudo --flag-que-nao-conheco config git push")).toBe(
      "sudo",
    );
    expect(programName("env -S nvim arquivo.ts")).toBe("env");
  });

  it("wrapper cujas flags consumiram tudo continua sendo o programa", () => {
    expect(programName("sudo -u app")).toBe("sudo");
  });

  it("bundle de flags curtas: só o último caractere pode levar valor", () => {
    // `sudo -Hu app make`: `-H` é booleana e `-u` leva valor.
    expect(programName("sudo -Hu app make")).toBe("make");
    // Com a flag de valor no meio, o resto do bundle seria o valor dela —
    // ambíguo demais para adivinhar.
    expect(programName("sudo -uH app make")).toBe("sudo");
  });
});

describe("continuação do shell (PS2)", () => {
  const emPS2 = {
    ...atPrompt,
    command: {
      command: "for i in 1 2 3; do",
      running: false,
      agent_match: false,
      continuation: true,
    },
  };

  it("o teclado continua na linha do TYBA", () => {
    // Ia para o terminal, e era o bug: em modo prompt o xterm fica escondido
    // atrás da lista de blocos. `ls \` + Enter e o teclado ia para um
    // terminal que ninguém via — a sessão parecia travada, sem onde digitar o
    // resto. A linha do TYBA só escreve no PTY, e é o shell, parado no `PS2`,
    // que junta o que chega ao comando pela metade.
    expect(keyboardOwner(emPS2)).toBe("tybaLine");
  });

  it("a linha diz que o shell espera o resto, e aceita o resto", () => {
    expect(lineState(emPS2)).toBe("continuation");
    expect(boxAcceptsTyping(lineState(emPS2))).toBe(true);
  });

  it("comando rodando ainda devolve o teclado ao terminal", () => {
    // O resto foi enviado e o comando começou: aí quem lê stdin é ele.
    expect(
      keyboardOwner({
        ...emPS2,
        command: { ...emPS2.command, running: true, continuation: false },
      }),
    ).toBe("terminal");
  });

  it("alt-screen ganha da continuação", () => {
    expect(lineState({ ...emPS2, altScreen: true })).toBe("app");
  });

  it("sem continuação, nada muda", () => {
    expect(lineState(atPrompt)).toBe("own");
  });
});

describe("boxAcceptsTyping", () => {
  const STATES: LineState[] = [
    "own",
    "waiting",
    "running",
    "continuation",
    "app",
    "off",
  ];

  it("a caixa aceita tecla enquanto o shell ainda está carregando", () => {
    // O bug que este predicado fecha: `waiting` é o intervalo entre a sessão
    // abrir e o shell reportar o primeiro prompt — 1,4 s no `.zshrc` do dono.
    // A caixa ficava desabilitada ali, e textarea desabilitada não dispara
    // `keydown`: o que se digitasse no primeiro segundo de cada sessão não
    // aparecia em lugar nenhum, e o Enter não fazia nada.
    expect(boxAcceptsTyping("waiting")).toBe(true);
    expect(boxAcceptsTyping("own")).toBe(true);
    // E `continuation`: o terminal está escondido, então o resto do comando
    // só tem a caixa para ser digitado.
    expect(boxAcceptsTyping("continuation")).toBe(true);
  });

  it("os outros continuam fechados, e cada um por um motivo", () => {
    // Não é uma lista de conveniência. `running`: quem lê o teclado é o
    // comando — é a regra que impede a caixa de engolir a senha do sudo.
    // `app`: a textarea nem está no DOM. `off`: o shell respondeu que NÃO
    // está em modo prompt, e a linha do TYBA não teria para onde enviar.
    expect(STATES.filter((state) => !boxAcceptsTyping(state))).toEqual([
      "running",
      "app",
      "off",
    ]);
  });

  it("estar na tela e aceitar tecla são perguntas diferentes", () => {
    // O par que se confundia num predicado só. `waiting` está montada E aceita
    // tecla; `running` está montada e NÃO aceita. Quem responder uma pela outra
    // volta a desabilitar o carregamento — ou, pior, libera a caixa no meio de
    // um comando que está lendo stdin.
    //
    // `off` saiu desta lista em 22/08: ele deixou de ter caixa montada. Não é
    // que passou a aceitar tecla — é que não há mais caixa ali. `continuation`
    // saiu por outro motivo: passou a aceitar, porque o resto do comando só
    // tem a caixa para ser digitado.
    const mountedButClosed = STATES.filter(
      (state) => boxIsMounted(state) && !boxAcceptsTyping(state),
    );
    expect(mountedButClosed).toEqual(["running"]);
  });
});

describe("boxIsMounted", () => {
  const STATES: LineState[] = [
    "own",
    "waiting",
    "running",
    "continuation",
    "app",
    "off",
  ];

  it("app de tela cheia e modo clássico trocam a caixa pela faixa", () => {
    // `off` entrou em 22/08. No modo clássico a caixa ficava montada e
    // desabilitada: ~36px de input morto no rodapé, com o prompt de verdade no
    // terminal logo acima. É a mesma saída que `app` já usava.
    expect(STATES.filter((state) => !boxIsMounted(state))).toEqual([
      "app",
      "off",
    ]);
  });

  it("a linha que não é minha ainda tem caixa na tela, com o rascunho dentro", () => {
    // O defeito que este predicado existe para impedir: o efeito que zerava a
    // altura medida da caixa saía por `state !== "own"` — e quatro dos cinco
    // estados que esse guarda pega mantêm a MESMA textarea montada. Colar três
    // linhas e ver a sessão reportar `running` colapsava a caixa para 28px com
    // o texto fora de vista, e nada o devolvia depois: altura inline não se
    // desfaz sozinha.
    const clearedByOldGuard = STATES.filter((state) => state !== "own");
    expect(clearedByOldGuard.filter(boxIsMounted)).toEqual([
      "waiting",
      "running",
      "continuation",
    ]);
  });

  it("voltar do app remonta a caixa, e caixa remontada nasce sem altura", () => {
    // A outra metade, e a razão de a medida ser refeita por `state`: em `app` a
    // textarea some do DOM, então o elemento que volta é OUTRO — o rascunho
    // continua no estado do React, mas a altura inline daquele elemento morreu
    // junto com ele.
    expect(boxIsMounted("app")).toBe(false);
    expect(boxIsMounted("own")).toBe(true);
  });
});

describe("promptModeEnabled", () => {
  it("vem ligada em instalação nova", () => {
    // Sem preferência gravada ninguém escolheu nada, e a escolha por omissão
    // tem de ser a que serve a quem acabou de instalar.
    expect(promptModeEnabled(null)).toBe(true);
    expect(promptModeEnabled(undefined)).toBe(true);
  });

  it("respeita quem já escolheu", () => {
    expect(promptModeEnabled("on")).toBe(true);
    expect(promptModeEnabled("off")).toBe(false);
  });

  it("valor estranho não desliga", () => {
    // `off` é a única resposta que desliga: preferência corrompida ou de uma
    // versão futura não pode devolver o usuário ao terminal cru em silêncio.
    expect(promptModeEnabled("")).toBe(true);
    expect(promptModeEnabled("talvez")).toBe(true);
  });
});

describe("commandPrefix", () => {
  it("devolve o que foi digitado quando o caret está na primeira palavra", () => {
    expect(commandPrefix("pn", 2)).toBe("pn");
  });

  it("é nulo depois do primeiro espaço", () => {
    // Passado o primeiro token, quem responde é caminho ou argumento. Consultar
    // a lista de comandos ali seria oferecer `pnpm` como valor de uma flag.
    expect(commandPrefix("git comm", 8)).toBeNull();
    expect(commandPrefix("ls ", 3)).toBeNull();
  });

  it("é nulo com a caixa vazia", () => {
    // Prefixo vazio casaria com tudo: os milhares de binários do $PATH
    // despejados no instante em que o cursor chega na linha.
    expect(commandPrefix("", 0)).toBeNull();
  });

  it("ignora o espaço à esquerda, que é o ignorespace", () => {
    // ` comando` é a convenção "não guarde isto no histórico" — o espaço é
    // sinal, não separador de token.
    expect(commandPrefix("  pn", 4)).toBe("pn");
  });

  it("olha o caret, não o fim do texto", () => {
    // Editar o começo de uma linha já escrita: o caret está no primeiro token
    // mesmo havendo mais palavras depois dele.
    expect(commandPrefix("pn install algo", 2)).toBe("pn");
  });
});

describe("enterAplicaSugestao", () => {
  it("com a lista aberta e um item escolhido, aplica", () => {
    expect(enterAplicaSugestao({ listaVisivel: true, selecionado: 0 })).toBe(true);
  });

  it("com a lista aberta e NADA escolhido, roda o que está escrito", () => {
    // Este é o caso que a lista automática cria. Ela abre sozinha, sem nada
    // marcado, e o Enter tem de rodar o comando digitado. Aplicar aqui seria o
    // app trocar o comando do usuário por outro no instante em que ele manda
    // executar — inaceitável num terminal.
    expect(enterAplicaSugestao({ listaVisivel: true, selecionado: -1 })).toBe(false);
  });

  it("com a lista fechada, roda", () => {
    expect(enterAplicaSugestao({ listaVisivel: false, selecionado: 0 })).toBe(false);
  });

  it("lista fechada com item marcado ainda roda", () => {
    // Estado que aparece ao fechar com Esc sem limpar o índice: o que manda é
    // a lista estar visível, não haver resquício de seleção.
    expect(enterAplicaSugestao({ listaVisivel: false, selecionado: 2 })).toBe(false);
  });
});

describe("qual cinza ganha: caminho ou argumento", () => {
  it("token comum: o que a base sabe do comando ganha do nome de pasta", () => {
    // `openssl s` estava virando `openssl skills/` porque existe uma pasta
    // `skills/` no home. O caminho é um palpite genérico sobre o diretório; a
    // base sabe que `openssl` tem `s_client`. Conhecimento específico ganha.
    expect(ghostDoToken("s", "kills/", "_client")).toEqual({
      texto: "_client",
      fonte: "argumento",
    });
  });

  it("token que JÁ é caminho: o caminho ganha de volta", () => {
    // Quem digitou `./s` ou `src/i` disse com todas as letras que quer um
    // arquivo. Ali o histórico completando outra coisa é que seria o palpite.
    for (const t of ["./s", "../s", "~/s", "/us", "src/i"]) {
      expect(ghostDoToken(t, "rc/", "ubcomando").fonte).toBe("caminho");
    }
  });

  it("sem argumento, o caminho continua servindo em token comum", () => {
    // O conserto do `openssl skills/` não pode desligar o cinza de caminho:
    // `cat R` → `README.md` é o caso que a completação de arquivo existe para
    // atender, e ali não há argumento nenhum competindo.
    expect(ghostDoToken("R", "EADME.md", "")).toEqual({
      texto: "EADME.md",
      fonte: "caminho",
    });
  });

  it("sem nenhum dos dois, não inventa cinza", () => {
    expect(ghostDoToken("x", "", "")).toEqual({ texto: "", fonte: null });
  });
});

describe("lineVisible", () => {
  const base = {
    kind: { type: "ssh", host_id: "h1" } as const,
    sessionIntegration: { state: "integrated", reason: "ok" } as const,
    promptMode: false,
    promptModePref: true,
    hookExpected: false,
  };

  it("a sessão ssh integrada mostra a linha quando o shell remoto reporta o prompt", () => {
    expect(lineVisible({ ...base, promptMode: true })).toBe(true);
  });

  it("antes disso não mostra nada na sessão remota", () => {
    // Local, a preferência adianta a linha porque o core sabe se o hook foi
    // injetado. Do outro lado do ssh essa resposta não existe, e adiantar
    // deixaria "Carregando o shell…" para sempre se o rc não subisse.
    expect(lineVisible(base)).toBe(false);
  });

  it("sessão ssh comum nunca mostra a linha", () => {
    expect(
      lineVisible({
        ...base,
        promptMode: true,
        sessionIntegration: { state: "plain", reason: "from-before" },
      }),
    ).toBe(false);
  });

  it("shell local continua adiantando a linha pela preferência", () => {
    expect(
      lineVisible({
        ...base,
        kind: { type: "shell" },
        sessionIntegration: undefined,
        hookExpected: true,
      }),
    ).toBe(true);
  });
});

describe("paneLine", () => {
  const idle = {
    command: null,
    running: false,
    agent_match: false,
    continuation: false,
  };
  const shellAt = (over: Partial<Parameters<typeof paneLine>[0]> = {}) =>
    paneLine({
      kind: { type: "shell" },
      reported: true,
      promptModePref: true,
      hookExpected: true,
      broadcasting: false,
      altScreen: false,
      command: idle,
      ...over,
    });

  it("cada painel decide com o estado da PRÓPRIA sessão", () => {
    // O defeito que a linha por painel elimina: um painel herdando o estado de
    // quem tem o foco. Dois painéis, um no prompt e outro com comando
    // rodando, saem com respostas diferentes — nenhum dos dois sabe qual está
    // ativo.
    const atPrompt = shellAt();
    const running = shellAt({ command: { ...idle, running: true } });
    expect(atPrompt).toEqual({ visible: true, state: "own", owns: true });
    expect(running).toEqual({ visible: true, state: "running", owns: false });
  });

  it("vim num painel não tira o teclado do vizinho", () => {
    // Tela cheia: a faixa some e o painel é todo do editor.
    expect(shellAt({ altScreen: true })).toEqual({
      visible: false,
      state: "app",
      owns: false,
    });
    expect(shellAt().owns).toBe(true);
  });

  it("antes do primeiro prompt a linha aparece esperando, sem o teclado", () => {
    expect(shellAt({ reported: undefined })).toEqual({
      visible: true,
      state: "waiting",
      owns: false,
    });
  });

  it("modo clássico: a linha fica na tela dizendo que está desligada", () => {
    expect(shellAt({ reported: false })).toEqual({
      visible: true,
      state: "off",
      owns: false,
    });
  });

  it("a rajada do broadcast devolve o teclado ao terminal", () => {
    expect(shellAt({ broadcasting: true }).owns).toBe(false);
  });

  it("sessão de agente não tem linha", () => {
    expect(
      shellAt({ kind: { type: "agent", runner: "claude-code" } as never })
        .visible,
    ).toBe(false);
  });

  it("claude rodando no painel: a faixa some e o painel é todo dele", () => {
    const running = { ...idle, running: true };
    expect(shellAt({ command: running, agentRunning: true }).visible).toBe(
      false,
    );
    expect(shellAt({ command: running }).visible).toBe(true);
  });

  it("agente em segundo plano não esconde o prompt do shell", () => {
    expect(shellAt({ agentRunning: true })).toEqual({
      visible: true,
      state: "own",
      owns: true,
    });
  });
});

describe("caretLine", () => {
  it("rascunho de uma linha é primeira e última ao mesmo tempo", () => {
    // É o que mantém ↑/↓ como sempre foram fora do multilinha.
    expect(caretLine("ls -la", 3)).toEqual({ first: true, last: true });
  });

  it("caixa vazia também", () => {
    expect(caretLine("", 0)).toEqual({ first: true, last: true });
  });

  it("na primeira linha de um multilinha", () => {
    expect(caretLine("ls \\\n  -la", 2)).toEqual({ first: true, last: false });
  });

  it("no meio: nem primeira nem última", () => {
    expect(caretLine("a\nb\nc", 2)).toEqual({ first: false, last: false });
  });

  it("na última linha", () => {
    expect(caretLine("a\nb\nc", 5)).toEqual({ first: false, last: true });
  });

  it("colado no fim da linha, antes do \\n, ainda é aquela linha", () => {
    expect(caretLine("a\nb", 1)).toEqual({ first: true, last: false });
  });

  it("newline no fim: depois dele é a última linha, vazia", () => {
    expect(caretLine("echo a\n", 7)).toEqual({ first: false, last: true });
    expect(caretLine("echo a\n", 6)).toEqual({ first: true, last: false });
  });

  it("caret fora do texto é preso nas bordas", () => {
    expect(caretLine("a\nb", 99)).toEqual({ first: false, last: true });
    expect(caretLine("a\nb", -1)).toEqual({ first: true, last: false });
  });
});

const hist = (command: string, failed = false) => ({
  command,
  failed,
  kind: "history" as const,
  label: null,
});

describe("menuItems — a ordem da lista que abre acima da linha", () => {
  const vazio = { commands: [], args: [], bins: [], paths: [] };

  it("o mais recente é o item 0, o que encosta na caixa", () => {
    // O core manda em ordem de execução, o mais novo primeiro.
    const items = menuItems({
      ...vazio,
      commands: [hist("lsd --tree"), hist("ls -la"), hist("docker ps")],
    });
    expect(items.map((i) => i.kind === "history" && i.command)).toEqual([
      "lsd --tree",
      "ls -la",
      "docker ps",
    ]);
  });

  it("repetição fica com a ocorrência mais recente", () => {
    const items = menuItems({
      ...vazio,
      commands: [hist("ls"), hist("pwd"), hist("ls")],
    });
    expect(items.map((i) => i.key)).toEqual(["history:ls", "history:pwd"]);
  });

  it("comando que só falhou não vira item", () => {
    const items = menuItems({ ...vazio, commands: [hist("lljh", true), hist("ls")] });
    expect(items.map((i) => i.key)).toEqual(["history:ls"]);
  });

  it("histórico primeiro, depois o que o comando oferece, binário e caminho", () => {
    const items = menuItems({
      commands: [hist("git checkout main")],
      args: [{ value: "checkout", description: "Switch branches" }],
      bins: [{ name: "gitk", kind: "path" }],
      paths: ["src/"],
    });
    expect(items.map((i) => i.kind)).toEqual(["history", "argument", "binary", "path"]);
  });
});

describe("menuRows", () => {
  it("desenha invertido: o item 0 é a ÚLTIMA linha, colada na caixa", () => {
    const rows = menuRows(["novo", "meio", "velho"]);
    expect(rows.map((r) => r.item)).toEqual(["velho", "meio", "novo"]);
    // O índice continua o lógico — é ele que a seleção usa.
    expect(rows.at(-1)).toEqual({ item: "novo", index: 0 });
  });
});

describe("moveSelection", () => {
  it("↑ da caixa escolhe o item colado nela", () => {
    expect(moveSelection(-1, "ArrowUp", 3)).toBe(0);
  });

  it("↑ sobe para o mais antigo e para no topo, sem dar a volta", () => {
    expect(moveSelection(0, "ArrowUp", 3)).toBe(1);
    expect(moveSelection(2, "ArrowUp", 3)).toBe(2);
  });

  it("↓ desce em direção à caixa e, do item 0, devolve o cursor à linha", () => {
    expect(moveSelection(2, "ArrowDown", 3)).toBe(1);
    expect(moveSelection(0, "ArrowDown", 3)).toBe(-1);
    expect(moveSelection(-1, "ArrowDown", 3)).toBe(-1);
  });

  it("lista vazia não tem o que escolher", () => {
    expect(moveSelection(-1, "ArrowUp", 0)).toBe(-1);
  });

  it("índice que sobrou de uma lista maior é preso no fim", () => {
    expect(moveSelection(9, "ArrowDown", 3)).toBe(2);
  });
});

describe("discoveryItems", () => {
  const a = (value: string, kind: string | null, uses = 0, alias: string | null = null) => ({
    value,
    description: null,
    uses,
    kind,
    alias,
  });

  it("o que você usa vem primeiro, mais usado no topo, depois subcomando, flag e valor", () => {
    const items = discoveryItems(
      [
        a("add", "subcommand"),
        a("--amend", "option"),
        a("main", null),
        a("commit", "subcommand", 3),
        a("status", "subcommand", 9),
      ],
      ["src/"],
    );
    expect(items.map((i) => i.key)).toEqual([
      "arg:status",
      "arg:commit",
      "arg:add",
      "arg:--amend",
      "arg:main",
      "path:src/",
    ]);
    expect(items[0]).toMatchObject({ group: "used" });
    expect(items[3]).toMatchObject({ group: "option" });
  });
});

describe("argValue", () => {
  it("com -- digitado escreve a forma longa", () => {
    expect(argValue({ value: "-m", alias: "--message" }, "--")).toBe("--message");
    expect(argValue({ value: "-m", alias: "--message" }, "-")).toBe("-m");
    expect(argValue({ value: "status" }, "")).toBe("status");
  });
});

describe("moveDiscovery", () => {
  it("↓ entra no primeiro, ↑ do primeiro volta para a caixa", () => {
    expect(moveDiscovery(-1, "ArrowDown", 3)).toBe(0);
    expect(moveDiscovery(2, "ArrowDown", 3)).toBe(2);
    expect(moveDiscovery(1, "ArrowUp", 3)).toBe(0);
    expect(moveDiscovery(0, "ArrowUp", 3)).toBe(-1);
  });
});

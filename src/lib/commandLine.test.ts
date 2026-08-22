import { describe, expect, it } from "bun:test";

import {
  boxAcceptsTyping,
  boxIsMounted,
  clearsDraft,
  controlBytes,
  ghostFor,
  isArrowKey,
  keyboardOwner,
  lineState,
  lineToken,
  pathToken,
  programName,
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

  it("não toma a linha de sessão ssh", () => {
    const ssh: OwnerInput = {
      ...atPrompt,
      kind: { type: "ssh", host_id: "h1" },
    };
    expect(keyboardOwner(ssh)).toBe("terminal");
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

  it("o teclado é do TERMINAL, não da linha do TYBA", () => {
    // O `PS2` não emite OSC nenhum, então `running` é false e sem o campo
    // `continuation` a linha se achava dona: o que fosse digitado ali viraria
    // submissão separada em vez do corpo do `for`.
    expect(keyboardOwner(emPS2)).toBe("terminal");
  });

  it("a linha diz que o shell espera o resto", () => {
    expect(lineState(emPS2)).toBe("continuation");
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
  });

  it("os outros continuam fechados, e cada um por um motivo", () => {
    // Não é uma lista de conveniência. `running` e `continuation`: quem lê o
    // teclado é o comando — é a regra que impede a caixa de engolir a senha do
    // sudo. `app`: a textarea nem está no DOM. `off`: o shell respondeu que NÃO
    // está em modo prompt, e a linha do TYBA não teria para onde enviar.
    expect(STATES.filter((state) => !boxAcceptsTyping(state))).toEqual([
      "running",
      "continuation",
      "app",
      "off",
    ]);
  });

  it("estar na tela e aceitar tecla são perguntas diferentes", () => {
    // O par que se confundia num predicado só. `waiting` está montada E aceita
    // tecla; `running` está montada e NÃO aceita. Quem responder uma pela outra
    // volta a desabilitar o carregamento — ou, pior, libera a caixa no meio de
    // um comando que está lendo stdin.
    const mountedButClosed = STATES.filter(
      (state) => boxIsMounted(state) && !boxAcceptsTyping(state),
    );
    expect(mountedButClosed).toEqual(["running", "continuation", "off"]);
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

  it("só o app de tela cheia troca a caixa pela faixa de uma linha", () => {
    expect(STATES.filter((state) => !boxIsMounted(state))).toEqual(["app"]);
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
      "off",
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

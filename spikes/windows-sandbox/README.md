# Spike — jaula de agente no Windows (sonda, não jaula)

Mede as três coisas que a ADR `2026-07-14-windows-token-restrito-nao-appcontainer`
exige **antes** de escrever uma linha de contenção. Cada sonda tem controle
positivo + teste enjaulado: um `FAIL` significa que a jaula bloqueou, não que o
setup quebrou.

Rode **na máquina Windows**, nesta ordem. São binários separados de propósito —
um erro de compilação num não derruba os outros.

```powershell
cd spikes\windows-sandbox

# SONDA A — a crítica: o gate por named pipe atravessa a jaula?
cargo run --bin gate

# SONDA C — AF_UNIX sobrevive? (decide transporte único cross-platform)
cargo run --bin afunix

# SONDA B — git escreve só no worktree? (precisa de git no PATH)
cargo run --bin worktree

# SONDA nodecheck — node.exe real inicia sob a jaula? (precisa de node no PATH)
cargo run --bin nodecheck

# Camada A — completude (cada uma com par positivo; precisam de node no PATH)
cargo run --bin jobobject    # kill-on-close: agente morre com o TYBA (paridade killpg)
cargo run --bin envjail      # env por allowlist: segredo do shell não vaza
cargo run --bin denyacl      # negar leitura de segredo ao agente (mecanismo: IL, não DACL-por-SID)
cargo run --bin mitigations  # quais process mitigations o node tolera (menos ACG, menos win32k)

# git roteado pelo core (precisa de git no PATH)
cargo run --bin gitshim      # o agente enjaulado delega git ao core pelo pipe herdado

# Camada B — usuário dedicado + WFP (PRECISA DE SHELL ELEVADO / UAC)
cargo run --bin layerb       # usuário dedicado isola leitura (dono lê, agente não)
cargo run --bin wfp          # rede por SID: loopback+RFC1918 negados, 443 externo livre

# ConPTY sob a jaula (RODAR COM CONSOLE REAL, não com stdout redirecionado)
Start-Process (Resolve-Path .\target\debug\conpty.exe) -Wait   # depois: cat $env:TEMP\tyba-conpty-result.txt
```

Cole a saída inteira das três de volta. O resultado vira dado na ADR e decide o
desenho da jaula — inclusive se o transporte do hook fica único ou se o Windows
precisa do dele.

## Verificação adversarial da suíte (2026-07-14)

A suíte inteira passou por um orquestrador de **verificação adversarial** (um
revisor cético por sonda, lendo o código + a saída, aplicando a regra "teste de
jaula sem par positivo e sem prova de que a jaula subiu não prova nada"). Achado:
as 7 sondas novas (worktree, nodecheck, jobobject, envjail, denyacl, mitigations,
gitshim) confirmaram; as **duas fundacionais tinham furos** e foram corrigidas:

- **gate** — a metade "por-nome negado" estava confundida: pipe de 1 instância já
  ocupada pelo pai dava `ERROR_PIPE_BUSY`, não `ACCESS_DENIED`. Reescrito com duas
  instâncias livres, DACL liberando Everyone + rótulo IL Medium no-write-up (só o
  IL Low nega), controle não-enjaulado que abre por nome, e leitura de
  `GetLastError` — agora prova `gle=5` (ACCESS_DENIED). **Re-verificado: CONFIRMED.**
- **afunix** — não tinha par positivo. Reescrito com controle não-enjaulado que
  conecta (exit 0), então a negação do enjaulado (exit 4) é atribuível à jaula.
  addrlen corrigido. **Re-verificado: CONFIRMED.**

Lição amarrada: no Windows a proteção da jaula (IPC por nome, leitura de segredo)
é por **Integrity Level**, não por DACL — vale para gate e denyacl.

## ConPTY sob a jaula (sonda conpty)

O agente roda num pseudo-console — é dele que vêm cores, prompt e `isatty`.
**Medido**: um `node.exe` sob token restrito (`jail_spec`), spawnado com o atributo
`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` via `CreateProcessAsUserW`, vê
`process.stdout.isTTY == true` (exit 7) — **o ConPTY sobrevive à jaula**. Par
positivo: o mesmo spawn sem jaula também dá isTTY=true. Consequência para o
produto: `sandbox/windows.rs` pode spawnar o agente **direto sob a jaula com o
ConPTY**, sem helper de passagem de handle. **Armadilha do harness**: ConPTY exige
um pai com console REAL — se o stdout do probe for redirecionado (pipe/arquivo), o
`isatty` do filho falha por artefato, não pela jaula (o controle fica vermelho e
avisa). Por isso a sonda grava o resultado em `%TEMP%\tyba-conpty-result.txt` e se
roda via `Start-Process`.

## O que cada PASS/FAIL significa

- **A / handle herdado usável + por-nome negado (com par positivo)** → o inbox
  recebe o `PreToolUse` do agente enjaulado pelo handle herdado; abrir o pipe pelo
  nome é negado por IL (`ACCESS_DENIED`, gle=5), enquanto um processo não-enjaulado
  abre o mesmo nome. É o cenário que a ADR aposta. Sem ele **não há produto**.
- **C / a jaula bloqueia AF_UNIX (com par positivo)** → o controle não-enjaulado
  conecta, o enjaulado não → o hook do Windows fica com o named pipe da sonda A
  (não é um transporte único cross-platform; é escolha de transporte por SO).
- **B / escreve dentro, negado fora, git commita** → o modelo de escrita
  (`WRITE_RESTRICTED` + SID por sessão) sustenta o worktree isolado com base de
  filesystem, não só política.
- **nodecheck / node inicia enjaulado?** → o `git.exe` sob a jaula dava
  `0xc0000142` (`STATUS_DLL_INIT_FAILED`); esta sonda mede o `node.exe` real (o
  agente) e **resolve a causa** com uma escada de ablação. Achado: o bloqueio era
  o **restricting set estreito** — a jaula original punha só um SID sintético,
  que não concede nada aos objetos que o init de um processo real toca (portas do
  csrss, `BaseNamedObjects` da sessão). A receita conhecida-boa (do
  `codex/windows-sandbox-rs` + Chromium) é pôr o **logon SID e o `Everyone`
  (S-1-1-0)** no restricting set. Ablação isola os fatores nesta máquina:
  - `Everyone` no restricting set: **necessário** (sem ele, volta o `0xc0000142`).
  - logon SID: presente na receita; sozinho não basta.
  - `lpDesktop=Winsta0\Default`: **dispensável** aqui (o codex seta por seguro).
  - **IL Low: mantido** — o node inicia em Low IL com o restricting set certo, então
    a jaula fica com a defesa-em-profundidade do IL baixo, não precisa subir a Medium.

  O SID sintético **permanece** no restricting set (é a chave da confinação de
  escrita ao worktree). A sonda `worktree` foi re-rodada com ESTE token real
  (logon+Everyone presentes) e a negação-fora **sobrevive** — IL Low + DACL
  seguram, o `Everyone` no restricting set não abre escrita ao diretório de fora.
  Receita da jaula: `lib::jail_spec(low_il=true)`.

## Camada A — completude (sondas jobobject/envjail/denyacl/mitigations)

Cada peça `DESENHADO` do tech-spec, medida na máquina com a receita real da jaula:

- **jobobject** → Job Object com `KILL_ON_JOB_CLOSE`: node enjaulado é atribuído,
  segue vivo com o job aberto (controle) e **morre quando o handle fecha**. Paridade
  real com o `killpg` do Linux (princípio #9). **PASS/PASS/PASS.**
- **envjail** → `CreateProcessAsUserW` com bloco de env montado da allowlist: o node
  vê só as vars liberadas + o marcador `TYBA_SANDBOX`, e um segredo no env do pai
  **não vaza**. **PASS/PASS/PASS.**
- **denyacl** → **achado que corrige o tech-spec**: um deny-ACE no SID sintético
  **não** nega leitura (o SID só conta no 2º access-check de *escrita*; o agente
  carrega a identidade do usuário na leitura). O mecanismo que **funciona** para
  negar leitura ao agente é o **rótulo IL `NO_READ_UP`** (`S:(ML;;NRNW;;;ME)`): o
  processo Low IL não lê o arquivo Medium-com-NR, o dono lê normal. Confinação de
  **leitura é por IL, não por DACL-de-SID**.
- **mitigations** → ablação de `PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY`. **Seguras**
  (aplicar): DEP, force-relocate, bottom-up ASLR, high-entropy ASLR, strict-handle-checks,
  extension-point-disable. **Quebram** (não aplicar): **ACG** (`PROHIBIT_DYNAMIC_CODE`,
  mata o JIT do V8 — `0x80000003`) e **win32k lockdown** (`0xc0000142`, severa a window
  station que o boot precisa — **corrige o tech-spec**, que listava win32k). **CIG**
  (block-non-MS) boota `node -e` puro, mas bloquearia `.node` addons nativos — precisa
  de spike com addon antes de adotar.

## git roteado pelo core (sonda gitshim)

- **Controle**: com a receita real da jaula, `git.exe` **inicia** (exit 0) — não dá
  mais `0xc0000142`. O motivo do shim muda de "git não roda" para **roteamento por
  controle**: git de escrita roda no core não-enjaulado, no worktree que ele controla.
- **Relay**: o processo enjaulado manda args de git pelo **pipe herdado**; o core roda
  git real fora da jaula e devolve a saída. Round-trip ponta a ponta. **PASS/PASS.**

## Camada B — usuário dedicado + WFP (sonda layerb) · PENDENTE DE ELEVAÇÃO

Camada B exige **admin (UAC)** — as sondas `layerb` e `wfp` são guardadas por
`is_elevated()`. **Medido em PowerShell admin:**

- **layerb / usuário dedicado** → cria `tyba-agent-*`, o **dono lê** um segredo do
  próprio perfil (controle), o agente rodando **como o usuário dedicado** é **negado**
  (`Access is denied.`), uninstaller remove o usuário. Leitura default-deny de verdade —
  por não-ter-acesso, não por deny-ACE enumerada.
- **wfp / rede por SID** → filtros WFP raw (sessão dinâmica) na camada
  `ALE_AUTH_CONNECT_V4` + `ALE_USER_ID` + endereço remoto, ação BLOCK para loopback +
  RFC1918. O **dono conecta** num listener de loopback e o **agente é bloqueado** no
  mesmo (`LOOP=blocked`), mas **alcança 443 externo** (`EXT=ok`). Sessão dinâmica remove
  os filtros ao fechar a engine (0 órfãos). Corta o agente de Docker/ollama/TYBA em
  localhost — o que a Camada A (blacklist) não faz.

## Aviso honesto

Isto é FFI de Win32 escrito **sem compilador Windows à mão** (autor no macOS).
Passou por `rustfmt` (sintaxe), não por `cargo check`. É esperado 1–2 rodadas de
ajuste de símbolo do `windows-sys` no primeiro `cargo run` — cole o erro que eu
corrijo. Isso é a sonda fazendo o trabalho dela: nada aqui se declara pronto sem
rodar de verdade.

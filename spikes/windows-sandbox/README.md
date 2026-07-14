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
```

Cole a saída inteira das três de volta. O resultado vira dado na ADR e decide o
desenho da jaula — inclusive se o transporte do hook fica único ou se o Windows
precisa do dele.

## O que cada PASS/FAIL significa

- **A / handle herdado usável + por-nome negado** → o inbox recebe o `PreToolUse`
  do agente enjaulado, e o único caminho até o pipe é o handle que herdou. É o
  cenário que a ADR aposta. Sem ele **não há produto** no Windows.
- **C / AF_UNIX atravessa** → o socket do hook pode ser o mesmo dos três SOs.
  Se `FAIL`, o Windows fica com o named pipe da sonda A (não é bloqueio, é escolha
  de transporte).
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

## Aviso honesto

Isto é FFI de Win32 escrito **sem compilador Windows à mão** (autor no macOS).
Passou por `rustfmt` (sintaxe), não por `cargo check`. É esperado 1–2 rodadas de
ajuste de símbolo do `windows-sys` no primeiro `cargo run` — cole o erro que eu
corrijo. Isso é a sonda fazendo o trabalho dela: nada aqui se declara pronto sem
rodar de verdade.

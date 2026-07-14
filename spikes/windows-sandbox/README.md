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

## Aviso honesto

Isto é FFI de Win32 escrito **sem compilador Windows à mão** (autor no macOS).
Passou por `rustfmt` (sintaxe), não por `cargo check`. É esperado 1–2 rodadas de
ajuste de símbolo do `windows-sys` no primeiro `cargo run` — cole o erro que eu
corrijo. Isso é a sonda fazendo o trabalho dela: nada aqui se declara pronto sem
rodar de verdade.

# TYBA

Seu ambiente, tudo conectado. Terminal opensource orientado a agentes de IA: orquestra múltiplas sessões de agentes (Claude Code, Codex) em git worktrees isolados, com inbox de aprovações, review de diff local e segurança como diferencial de produto.

## Stack

- **Desktop**: Tauri 2 (Rust core + webview)
- **UI**: React + TypeScript, Tailwind v4, shadcn/ui, Phosphor Icons
- **Terminal**: xterm.js (não reimplementar parser ANSI nem renderer)
- **PTY**: portable-pty (Rust)
- **Persistência**: SQLite local (rusqlite)
- **Git**: shell-out para o binário `git` (não usar git2/gitoxide no MVP)
- **Runtime/PM JS**: Bun (bun install, bunx, bun run — nunca pnpm/npm)

## Princípios de arquitetura (não negociáveis)

1. **Todo estado vive no Rust core.** O webview é burro: renderiza o que recebe via eventos e emite intenções via commands. Nenhuma lógica de sessão/git/status no lado React.
2. **Spawn de processos de agente SEMPRE atrás da trait `Sandbox`** (mesmo que a implementação inicial seja passthrough). Nunca `Command::new` cru espalhado no código para agentes.
3. **PTY output com batching**: acumular chunks e flushar para o webview a cada ~8-16ms. Nunca um evento IPC por chunk.
4. **Ações vermelhas nunca são automáticas**: `git push`, `gh pr create`, `sudo`, rede, escrita fora do worktree, `rm -rf`. Hard-coded no runner, não configurável para "sempre permitir".
5. **Push para main/master de sessão de agente é recusado pelo core.** Sempre.
6. **Env filtrado para agentes**: sessões de agente recebem env por allowlist (config por repo), nunca o env completo do shell do usuário.
7. **Diff de sessão contra base fixa**: `git diff <base_sha>..HEAD` dentro do worktree, com `base_sha` salvo na criação — é o base congelado que dá a semântica de three-dot (a sintaxe `..` é two-dot); nunca comparar com o estado atual da main.
8. **Output de git sempre com `-z`, `--no-color` e `-c core.quotePath=false`.** Parsing NUL-separated.
9. **Kill de sessão mata o process group inteiro** (`killpg`), não só o processo pai.
10. **Secrets nunca em log/scrollback persistido**: redação de padrões (AWS keys, JWT, `sk-...`) antes de gravar no SQLite.

## Estrutura

```
tyba/
├── src-tauri/src/
│   ├── session/    # SessionManager, lifecycle, store.rs (SQLite)
│   ├── pty/        # pool, batching
│   ├── worktree/   # git ops (add, diff, merge, gc de órfãos)
│   ├── agent/      # trait AgentRunner + claude_code.rs, codex.rs
│   ├── status/     # StatusDetector (stream-json + OSC 133 + heurística)
│   └── sandbox/    # trait Sandbox — Seatbelt (macOS) + bwrap/seccomp (Linux) reais; Windows Camada A (token restrito + ConPTY)
├── src/            # React: Terminal.tsx, Inbox.tsx, DiffView.tsx
└── docs/           # ARCHITECTURE.md, SECURITY.md, ROADMAP.md
```

## Docs de referência

- `docs/ARCHITECTURE.md` — modelo de dados, IPC, ciclo de vida de sessão, diff local
- `docs/SECURITY.md` — modelo de ameaça, classificação de risco de comandos, regras
- `docs/ROADMAP.md` — fases do MVP e ordem de construção

Leia o doc relevante antes de implementar qualquer módulo.

## Convenções

- Código e identificadores em inglês; docs e mensagens de UI em pt-BR (i18n depois).
- Commits: conventional commits (`feat:`, `fix:`, `refactor:`).
- Testes: unit para parsers (diff, OSC 133, stream-json) são obrigatórios — são a parte mais frágil.
- **Nunca commitar secrets. O repo já é público** — não há mais janela entre commitar e vazar: `git push` é publicação, e o que sobe fica legível na hora. Apagar depois não desfaz, porque o GitHub continua servindo objeto inalcançável por SHA e fork, clone e cache de terceiros não voltam atrás. O projeto em si não tem segredo (é um app desktop, sem backend); os riscos concretos são três:
  - **Fixtures de teste são sempre sintéticas.** Scrollback, transcript de agente e output de terminal se escrevem à mão — nunca se copia uma sessão real de `~/.claude/projects` ou `~/.codex/sessions` para reproduzir um bug. Um terminal grava tudo que o dono digitou; a fixture "só pra testar" carrega o token junto.
  - **O que o `push` leva junto conta como publicado.** Não basta olhar o diff: hook de `pre-push` empurra o que quiser, e o autor do commit não vê. Foi assim que o hook do Entire publicou 1,42 GiB de transcript de sessão na branch `entire/checkpoints/v1` — todo prompt, todo arquivo lido e toda saída de comando —, por 24 dias, sem aparecer em nenhum diff. Desligado com `strategy_options.push_sessions: false` em **dois** arquivos, e os dois são necessários: `.entire/settings.json` é versionado — vale como política do repo, mas viaja com a branch, então `git checkout` numa branch anterior ao desligamento traz a configuração antiga de volta e o `push` seguinte republica tudo (foi o que aconteceu, uma vez, depois de já ter sido corrigido); `.entire/settings.local.json` é ignorado pelo git, então não troca junto com a branch e é o que realmente segura. A lição maior é conferir o que **mais** sai, não só o que se escreveu.
  - **Chaves de assinatura vivem em GitHub Secrets** (Fase 6): certificado Developer ID + senha, credenciais de notarização, e a chave privada do auto-update. Essa última é a mais perigosa — quem a tiver publica um "update" que a máquina do usuário instala sozinha.

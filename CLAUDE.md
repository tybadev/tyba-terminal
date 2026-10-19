# TYBA

Seu ambiente, tudo conectado. Terminal opensource orientado a agentes de IA: o agente (Claude Code, Codex) roda livre como num terminal comum, e o TYBA é o melhor observador dele — monitor e histórico da conversa, subagentes, notificação quando o agente precisa de você, review de diff local, worktrees para rodar vários em paralelo — com um guarda-corpo fixo só para o irreversível.

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
2. **Spawn de processos de agente SEMPRE atrás da trait `Sandbox`.** No caminho padrão (agente livre, digitado ou aberto pelo TYBA) a implementação é o passthrough; Seatbelt, bwrap e a jaula do Windows ficam para um modo autônomo futuro. Nunca `Command::new` cru espalhado no código para agentes.
3. **PTY output com batching**: acumular chunks e flushar para o webview a cada ~8-16ms. Nunca um evento IPC por chunk.
4. **Agente livre, TYBA observador.** Claude e codex — digitados no pane ou abertos pelo TYBA, em worktree ou não — rodam livres: quem pede permissão é o próprio CLI (prompt nativo do Claude Code; sandbox e aprovação do Codex), uma vez só. O TYBA não pergunta de novo, não abre card e não empilha jaula; observar nunca bloqueia o agente. Ver `docs/SECURITY.md`.
5. **Guarda-corpo fixo, em todo SO:** `git push` para main/master e merge de PR/MR (`gh pr merge`, `glab mr merge` e o mesmo por API) por agente são recusados — sem perguntar, hard-coded, não configurável. Falha fechado: sem saber o destino do push, recusa.
6. **Env do agente é o env do usuário:** o do shell do pane no agente digitado; o env completo do shell de login no agente que o TYBA abre. Jaula e env por allowlist só num modo autônomo futuro.
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

vault_path: ~/swell-system/swell-docs (pasta do projeto: `tyba/`)

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
  - **O que o `push` leva junto conta como publicado.** Não basta olhar o diff: hook de `pre-push` empurra o que quiser, e o autor do commit não vê. Foi assim que o hook do Entire publicou 1,42 GiB de transcript de sessão na branch `entire/checkpoints/v1` — todo prompt, todo arquivo lido e toda saída de comando —, por 24 dias, sem aparecer em nenhum diff. Desligar só pela configuração não bastou: `.entire/settings.json` é versionado e viaja com a branch, então um `git checkout` numa branch anterior ao desligamento trouxe a configuração antiga de volta e o `push` seguinte republicou tudo — uma vez, depois de já ter sido corrigido. Por isso o Entire **saiu do projeto** (2026-09-19): sem `.entire/`, sem hooks em `.claude/settings.json`, sem o CLI instalado. Não reinstale nem rode `entire enable` aqui; branch antiga ainda traz os arquivos de volta, mas os hooks dela não fazem nada sem o binário. A lição maior é conferir o que **mais** sai, não só o que se escreveu.
  - **Chaves de assinatura vivem em GitHub Secrets** (Fase 6): certificado Developer ID + senha, credenciais de notarização, e a chave privada do auto-update. Essa última é a mais perigosa — quem a tiver publica um "update" que a máquina do usuário instala sozinha.

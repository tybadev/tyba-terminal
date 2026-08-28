---
name: release-changelog
description: Escreve a entrada do changelog no tyba-site para uma versão nova do TYBA. Acionar SEMPRE que for cortar um release (git tag vX.Y.Z), quando o usuário disser "vamos lançar", "sair a versão", "cortar release", "escrever o changelog", ou depois de mergear um conjunto de PRs que fecha uma versão. O changelog é PARTE do release — o toast de versão nova na app aponta para essa página, então publicar sem ela deixa o usuário num link vazio.
---

# Changelog da versão nova no site

O TYBA avisa o usuário quando sai versão nova (`src-tauri/src/update/`), e o botão do aviso abre **`https://tyba.dev/{locale}/changelog`** — não a tela de release do GitHub. Ou seja: **a página do changelog é o que o usuário vê quando atualiza.** Se ela não tiver a versão, ele clica e não encontra nada.

Por isso esta skill existe: escrever o changelog não é o passo depois do release, é parte dele.

## Onde mora

- **Dados**: `../tyba-site/src/lib/changelog.ts` — array `CHANGELOG`, **mais recente primeiro**.
- **Página**: `src/app/[lang]/changelog/page.tsx` (só renderiza; não mexer pra adicionar versão).
- Cada entrada: `{ version, date, items: { 'pt-br': string[], en: string[] } }`.
- `date` no formato `'2026-07'` (ano-mês, sem dia).

## Como levantar o que entrou

Nunca escreva de memória. Levante do repo `tyba-terminal`.

Todo caminho nesta skill é relativo à **raiz do `tyba-terminal`**, e é de lá que os
comandos rodam — `tyba-site` é uma checkout irmã, em `../tyba-site`. Se você já
navegou para o site, volte antes de levantar os commits, senão o `git log` abaixo
responde pelo repo errado e a versão sai com a lista de mudanças de outro projeto.

```bash
git fetch --tags
git log <última-tag>..main --merges --oneline
gh pr list --repo tybadev/tyba-terminal --state merged --limit 50 \
  --json number,title,mergedAt,body --jq '.[] | "\(.number) \(.title)"'
```

Se não houver tag anterior (primeiro release), pegue tudo o que está na `main`.

## A regra de voz — e ela é a parte difícil

O arquivo já traz a regra, e ela vale mais que qualquer template:

> **"A mesma regra do resto do site: só entra aqui o que roda de verdade."**

Disso decorre:

- **Título de PR não é item de changelog.** `fix(sandbox): worktree isolado no Linux morria enterrado pelo próprio shadow` é ótimo pra quem revisa código e péssimo pro usuário. O item é: *"Sessão isolada em worktree volta a funcionar no Linux."*
- **Escreva o que o usuário ganha, não o que o código faz.** Ninguém liga pra `--tmpfs` na ordem errada; ligam pra "o agente agora abre".
- **Nada de "melhorias diversas", "vários bugs corrigidos", "refatorações internas".** Se não dá pra dizer o que mudou pra quem usa, não entra.
- **Não anuncie o que não roda.** Feature atrás de flag, código mergeado mas não ligado, ou coisa que só existe em uma plataforma sem dizer qual — fora, ou com a ressalva explícita.
- **Termos de domínio ficam em inglês** nos dois idiomas (branch, diff, worktree, merge) — é vocabulário técnico, não texto de UI.
- Poucos itens e densos. Olhe as entradas existentes: 2 a 4 itens por versão, cada um uma frase que se sustenta sozinha.

## O que fazer

1. **Levante os PRs** desde a última tag (comandos acima). Leia os títulos **e os corpos** — o corpo costuma dizer o impacto no usuário; o título diz o mecanismo.
2. **Agrupe por impacto**, não por arquivo. Três PRs que juntos consertam o Linux viram um item, não três.
3. **Escreva pt-br e en** — não é tradução literal; é a mesma ideia nas duas línguas, cada uma soando natural.
4. **Insira no topo** do array `CHANGELOG` em `tyba-site/src/lib/changelog.ts`. A versão tem que ser **exatamente** a da tag (o gate de versão da CI já garante que a tag bate com `tauri.conf.json`, `package.json` e `Cargo.toml` — o changelog usa a mesma).
5. **Rode os gates do site** antes de abrir PR (veja `tyba-site/CLAUDE.md`; tipicamente `bun run typecheck` / `biome`).
6. **Abra PR no `tyba-site`** — conventional commit, mensagem em pt-BR, sem atribuição a IA.

## Antes de dar por pronto

- A versão do changelog **bate com a tag** que vai ser publicada?
- Algum item descreve mecanismo em vez de benefício?
- Algum item fala de algo que **não roda** nesta versão?
- Os dois idiomas dizem a mesma coisa?
- O link do toast (`https://tyba.dev/{locale}/changelog`) vai encontrar essa versão quando o usuário clicar?

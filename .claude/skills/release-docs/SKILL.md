---
name: release-docs
description: Atualiza a doc de produto do TYBA no Mintlify (docs-site/) para as features com superfície de usuário que entraram numa versão nova. Acionar SEMPRE que for cortar um release (git tag vX.Y.Z), quando o usuário disser "vamos lançar", "sair a versão", "cortar release", ou depois de mergear PRs que ligam feature nova pro usuário. A doc de produto é PARTE do release — assim como o changelog do site. É a companheira de release-changelog, mas para a documentação de uso, não para o "o que mudou".
---

# Doc de produto da versão nova no Mintlify

O TYBA tem duas superfícies de doc pública, e elas não são a mesma coisa:

- **Changelog** (`tyba-site`, skill `release-changelog`): *o que mudou* nesta versão. Efêmero, cronológico, uma frase por item.
- **Doc de produto** (`docs-site/`, esta skill): *como usar* a feature. Permanente, organizada por assunto, é onde o usuário aprende a operar o app.

Cortar um release ligando uma feature sem doc de produto entrega ao usuário algo que ele não sabe usar — e, pior, não descobre que existe, porque a navegação da doc não tem sequer o grupo. Não é hipótese: **o file explorer inteiro (fatias 1, 2, 3, 4a) foi para produção sem uma linha em `docs-site/`** — quatro PRs de feature de usuário, invisíveis na doc oficial. Esta skill existe para que isso pare de acontecer.

## Onde mora

- **Fonte**: `tyba-terminal/docs-site/` — MDX, no próprio repo. Publica no Mintlify a partir daí; a fonte de verdade é o repo, não o editor do Mintlify.
- **Navegação e idiomas**: `docs-site/docs.json` — array `navigation.languages`, um bloco por idioma.
- **Trilíngue, com paridade**: `pt-br/` (default), `en/`, `es/`. Toda página nasce nos **três** idiomas e entra nos **três** blocos de `docs.json`. Página só em um idioma é bug de navegação — o seletor de idioma leva a um 404.
- **Frontmatter de cada `.mdx`**: `title` + `description`. Componentes Mintlify (`<Note>`, `<Warning>`, `<Steps>`, tabelas) já são o vocabulário do site — use os mesmos das páginas vizinhas.
- Edição direta no deployment (sem passar pelo repo) existe via o MCP do Mintlify (`checkout` → `edit_page`/`write_page` → `save`), mas só para hotfix de doc fora de release; **no fluxo de release, a doc muda no repo** e sobe com o código.

## O que entra na doc de produto (e o que não)

Nunca escreva de memória. Levante do repo o que a versão liga:

```bash
git -C /Users/guilherme/swell-system/tyba-terminal fetch --tags
git -C /Users/guilherme/swell-system/tyba-terminal log <última-tag>..main --merges --oneline
gh pr list --repo tybadev/tyba-terminal --state merged --limit 50 \
  --json number,title,body --jq '.[] | "\(.number) \(.title)"'
```

A régua de corte é diferente da do changelog:

- **Entra**: toda mudança que o usuário **opera** — um painel novo, um atalho, um fluxo, um card de consentimento, uma tela de configuração, um comando na paleta. Se o usuário toca, precisa saber usar.
- **Não entra**: mudança máquina-a-máquina sem tela (um parser interno, um gate de CI, um refactor de sandbox que não muda o que o usuário faz). Isso é changelog, no máximo — não é doc de produto.
- **Feature atrás de flag ou meio-ligada**: fora, ou com a ressalva explícita da plataforma. A doc só descreve o que **roda de verdade** nesta versão — mesma regra do changelog e do resto do site.

## A voz — e ela é a parte difícil

> **Documente o que o usuário faz, não o que o código faz.**

Disso decorre:

- **Título de PR não é título de página.** `feat(lsp): download gerenciado com checksum pinado` é ótimo pro reviewer e inútil pro usuário. A página é *"Language servers"* e a seção é *"Quando o TYBA baixa um server para você"*.
- **Comece pela tarefa, não pela arquitetura.** O usuário quer completar código, não saber que o server roda enjaulado. A jaula entra quando explica um comportamento que ele vê (por que pediu consentimento, por que não tem rede) — não como abertura.
- **Termos de domínio em inglês** nos três idiomas (branch, diff, worktree, LSP, merge) — vocabulário técnico, não texto de UI.
- **Densidade das páginas vizinhas.** Olhe uma página existente do mesmo grupo antes de escrever: mesmo tamanho de seção, mesmos componentes, mesma profundidade. A doc lê como um sistema só.
- **Paridade real entre idiomas**: mesma ideia, cada língua soando natural — não tradução literal do pt-BR.

## O que fazer

1. **Levante os PRs** desde a última tag (comandos acima). Leia título **e corpo** — o corpo diz o impacto no usuário.
2. **Separe** o que tem superfície de usuário do que é interno (régua acima). Só o primeiro grupo vira doc.
3. **Ache o lugar na navegação.** A feature encaixa num grupo existente (`docs.json`) ou precisa de um grupo novo? Grupo novo entra nos três idiomas, na mesma posição relativa.
4. **Escreva as páginas nos três idiomas** (`pt-br/`, `en/`, `es/`) e **registre cada uma nos três blocos** de `docs.json`. Uma página que existe no disco mas não está no `docs.json` não aparece na navegação.
5. **Rode os gates do site** antes de abrir PR (veja `docs-site/` — tipicamente lint de MDX / `mintlify broken-links` se disponível; no mínimo confira que todo path do `docs.json` existe nos três idiomas).
6. **Abra PR no `tyba-terminal`** (a doc vive no mesmo repo do app) — conventional commit `docs(...)`, mensagem em pt-BR, sem atribuição a IA. A doc entra junto com o código da feature quando possível; num release que documenta feature já mergeada, é um PR de doc próprio.

## Antes de dar por pronto

- Toda feature de usuário que esta versão liga tem página? (cruze a lista de PRs com a navegação)
- As páginas existem nos **três** idiomas e estão nos **três** blocos de `docs.json`?
- Algum path no `docs.json` aponta pra arquivo que não existe? (404 na navegação)
- Alguma página descreve arquitetura em vez de tarefa? Fala de algo que **não roda** nesta versão?
- O grupo novo (se houve) entrou na mesma posição nos três idiomas?

## Dívida conhecida de backfill

O file explorer (fatias 1–4a + 3.1: painel Arquivos, tree, viewer, edição + file ops, fuzzy, decorações git, LSP local enjaulado + download gerenciado, remoto SSH) **não tem nenhuma página em `docs-site/`**. É a maior lacuna atual e um grupo "Arquivos" inteiro a criar nos três idiomas. Não é trabalho de um release pontual — é tarefa dedicada. Enquanto não for feito, todo release que tocar o file explorer herda essa dívida; registre no changelog o que a versão liga, mas a doc de produto do épico continua devendo até o backfill.

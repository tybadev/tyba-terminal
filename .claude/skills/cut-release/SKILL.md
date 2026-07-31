---
name: cut-release
description: Corta um release do TYBA (tag vX.Y.Z) do começo ao fim — ensaio, changelog, tag, aprovação e publicação, incluindo anexar o macOS depois que o certificado da Apple chegar. Acionar SEMPRE que o usuário disser "vamos lançar", "cortar release", "publicar a versão", "subir a tag", ou pedir para gerar binários. O pipeline tem armadilhas que já quebraram três vezes; esta skill existe para que ninguém as redescubra.
---

# Cortar um release do TYBA

O `release.yml` **não é código que você lê e entende**: é processo. Ele já quebrou três vezes em ensaio e três vezes no dia de lançar, sempre por um motivo que parecia óbvio depois. Siga a ordem.

## A regra que explica quase tudo

> **Um binário que ninguém consegue instalar é pior que nenhum binário.**

Daí decorrem as duas travas que **não** devem ser afrouxadas:

- **macOS sem certificado não sai.** O job assina e verifica com `codesign` + `spctl`. Sem o certificado Developer ID, o `.dmg` é ad-hoc: o Gatekeeper diz "app danificado" e o usuário conclui que o produto está quebrado. **Isso vale para Intel também** — a assinatura é do app, não da arquitetura.
- **A `main` só entra por PR com os 4 gates verdes**, e a tag roda os gates de novo (uma tag pode apontar para qualquer commit).

## Antes de qualquer coisa

1. **A versão bate nos três manifestos?** `src-tauri/tauri.conf.json`, `package.json`, `src-tauri/Cargo.toml`. O job `version` recusa a tag se divergir — e é ele que impede publicar um `Tyba_0.1.0.dmg` dentro de um release `v0.2.0`.
2. **O changelog do site já tem essa versão?** O toast de versão nova da app e a página de download **apontam para `tyba.dev/{locale}/changelog`**. Publicar sem o changelog deixa o usuário num link que não tem a versão dele. Use a skill `release-changelog`.
3. **A doc de produto do Mintlify cobre o que essa versão liga?** As features com superfície de usuário que entraram desde a última tag têm página em `docs-site/` (pt-BR/en/es)? Um release que liga uma feature sem doc entrega ao usuário algo que ele não sabe usar nem descobre existir — foi o que aconteceu com o file explorer inteiro (quatro fatias invisíveis na doc). Use a skill `release-docs`.
4. **`vars.RELEASE_PLATFORMS` está certo?** (`gh variable list`)
   - `linux` → a tag publica só Linux. É o valor correto **enquanto o certificado da Apple não existir**.
   - `all` → Linux + macOS (Apple Silicon e Intel). Só depois que os secrets da Apple estiverem no Environment `release`.

   Sem essa variável, `git push --tags` buildaria macOS, falharia por falta de certificado e **levaria o Linux junto**.

## Ensaiar antes (não pule)

Os gates da CI **não exercitam o `release.yml`** — ele só roda de verdade numa tag ou num dispatch. Qualquer mudança em action, bump de dependência ou passo novo **passa verde em todos os PRs e só quebra no dia**. Foi assim que os bumps do Dependabot entraram sem nunca terem sido executados.

**Ensaie com uma tag `dry-v<versão>`.** Ela roda o pipeline **inteiro — `publish` incluído** — contra um release descartável, confere que os assets subiram, e apaga release e tag no fim.

```bash
git tag dry-v0.1.1        # a MESMA versão dos manifestos: o job `version` roda de verdade
git push origin dry-v0.1.1
gh run watch
```

Sufixo para reensaiar sem colidir com uma tag que não foi limpa: `dry-v0.1.1-2`.

> [!danger] O ensaio antigo (dispatch na `main`) PARAVA antes do `publish`
> `github.ref` era `refs/heads/main`, o `if` do job `publish` não casava, e ele **nem começava**. Ou seja: o ensaio exercitava tudo **menos** o único passo que nunca tinha rodado. Foi assim que a `v0.1.0` queimou — e a tag `v*` não se apaga (ruleset, e é para não apagar mesmo).
>
> **Um passo que só roda no dia do release não é um passo testado — é uma aposta.**

O dispatch (`gh workflow run release.yml --ref main -f platforms=linux`) continua existindo e ainda serve para checar só o build, mais rápido. Mas **ele não prova o `publish`**. Antes de cortar tag real, o ensaio que vale é o `dry-v*`.

O ensaio roda no environment **`dry`**: sem secret, sem revisor, aceitando só `dry-v*`. Ele **nunca** vê o certificado da Apple — e não é o YAML que garante isso, é a política de ref do environment (`release` só aceita `v*`, `dry` só aceita `dry-v*`). Quem pode ensaiar não pode assinar.

ADR: `tyba/decisions/2026-07-14-tag-dry-ensaia-o-publish`.

## Cortar

```bash
git checkout main && git pull --ff-only
git tag v0.1.0
git push origin v0.1.0
```

O que acontece, em ordem:

1. `gates` → `version` (tag × manifestos) → `matrix` (lê `RELEASE_PLATFORMS`).
2. **Pausa ou não pausa, conforme a plataforma** — e a diferença importa:
   - **`macos` → PAUSA.** O build vai para o Environment `release`, que exige **revisor humano (o dono)**. Aparece como *"Review deployments"* na aba Actions. **Não é bug — é a trava funcionando, e ninguém além do dono aprova.**
   - **`linux`/`windows` → NÃO pausa.** Eles buildam no Environment **`release-unsigned`**, que não tem revisor.

   Não é inconsistência: **o revisor protege o segredo, não a publicação.** Quem aprova o `release` está liberando o uso do certificado Developer ID; o `release-unsigned` não tem segredo nenhum a proteger. Quem protege a publicação é o **rascunho** do passo 4.

   Enquanto o certificado da Apple não existir, **nenhum release pausa** — não fique esperando um botão que não vai aparecer. (Verificado na v0.1.3: esta página afirmava a pausa como incondicional e fez a sessão anunciar ao dono uma trava que não existia naquele caminho.)
3. Build → checksums → attestation de procedência.
4. `publish` cria o release como **rascunho**, de propósito: o dono confere os artefatos e publica na mão.

## Depois: anexar o macOS quando o certificado chegar

Os secrets da Apple vão para o **Environment `release`**, nunca para Actions secrets do repo — senão a trava do revisor é contornada.

```bash
gh variable set RELEASE_PLATFORMS --body all
gh workflow run release.yml --ref v0.1.0 -f platforms=macos
```

Disparar **na tag** faz `github.ref` ser `refs/tags/v0.1.0`: assina, verifica e publica. O passo de publish faz `upload --clobber` numa release que já existe — os `.dmg` entram **na mesma release**, sem recriar nada.

## Armadilhas que já custaram caro

- **Verde que mente.** Já aconteceu duas vezes: teste de sandbox passando com a jaula quebrada, e o release voltando `success` com o build skipado em cascata (um `if` de tag no job `version` fazia `build` skipar junto). Sempre pergunte: *passou porque funcionou, ou porque nem chegou a rodar?*
- **O `publish` já quebrou por causa de um glob.** Ele globava `artifacts/*.dmg` mesmo num release só de Linux — o glob não expandia e o `gh` morria com "no such file". Hoje a lista sai de um `find`, e um release **sem instalador nenhum falha alto**. Se mexer nesse passo, ensaie com `dry-v*` — que é exatamente o que passou a existir para isso.
- **O input `dry_run` era decorativo — foi REMOVIDO.** Estava declarado e não era lido em lugar nenhum: um botão que prometia ensaiar e não ensaiava. Quem decide tudo é o `github.ref`.
- **Ensaio só sai da `main` ou de tag.** Os Environments recusam branch arbitrária — o job nem começa, zero steps. É o custo consciente da proteção.
- **O `publish` agora CONFERE o que publicou** — consulta os assets do release e falha se faltar algum, ou se algum vier com tamanho zero (upload truncado volta `success` e só quebra na mão de quem baixa). "Não deu erro" não é prova.
- **Nunca gere a chave de assinatura de update no CI.** ADR aceita: o app verifica com a chave pública embutida, então canal e chave são defesas independentes; pôr a chave no CI colapsa as duas numa só. E o updater **não tem revogação**.

## Fechar

- Publicar o rascunho no GitHub (o dono decide o momento).
- **Acrescentar a versão em `tyba-site/src/lib/versions.ts`** — a página de versões antigas (`/{locale}/versions`) é **estática, mantida à mão**, exatamente como o changelog: sem fetch, sem rate limit, sem fallback para manter. Gere a entrada do release **que já existe**, nunca derive URL por padrão de nome:

  ```bash
  gh release view v0.1.3 --json publishedAt,assets \
    --jq '{version:"0.1.3", date:(.publishedAt[0:7]),
           assets:[.assets[]|{name,url:.url,size}]}'
  ```

  O nome do bundle é decidido pelo Tauri e já mudou entre versões: adivinhar o padrão gera link 404 silencioso, que é a mesma classe de mentira que o `LAST_KNOWN_RELEASE` existe para evitar.
- **Gravar a versão publicada em `tyba-site/src/lib/release.ts` (`LAST_KNOWN_RELEASE`)** — versão + plataformas que de fato saíram. **Este passo não é cosmético.** O site lê a Releases API sem token, e a Vercel roda em IP compartilhado: sob rate limit o fetch falha, e sem essa memória a página conclui *"nenhum binário publicado"* — ou seja, no pico de tráfego ela diz ao usuário que o download não existe. A regra: **o site pode não saber que saiu versão nova; nunca pode dizer que não existe binário quando existe.**
- Conferir que `tyba.dev/{locale}/download` já mostra a versão (a página lê a Releases API com cache de 1h — pode levar até uma hora, ou force um redeploy).
- Conferir que o changelog do site tem a versão publicada, **sem o selo de pré-release** (ele some sozinho quando o release existe).

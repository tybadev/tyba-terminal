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
3. **`vars.RELEASE_PLATFORMS` está certo?** (`gh variable list`)
   - `linux` → a tag publica só Linux. É o valor correto **enquanto o certificado da Apple não existir**.
   - `all` → Linux + macOS (Apple Silicon e Intel). Só depois que os secrets da Apple estiverem no Environment `release`.

   Sem essa variável, `git push --tags` buildaria macOS, falharia por falta de certificado e **levaria o Linux junto**.

## Ensaiar antes (não pule)

Os gates da CI **não exercitam o `release.yml`** — ele só roda de verdade numa tag ou num dispatch. Isso significa que qualquer mudança em action, bump de dependência ou passo novo **passa verde em todos os PRs e só quebra no dia**. Foi assim que os bumps do Dependabot entraram sem nunca terem sido executados.

```bash
gh workflow run release.yml --ref main -f platforms=linux
gh run watch
```

Rodando na `main` (não numa tag), `github.ref` não é `refs/tags/v*`: não assina, não verifica e **não publica**. É ensaio de verdade — build completo, os 3 formatos de Linux e a attestation.

## Cortar

```bash
git checkout main && git pull --ff-only
git tag v0.1.0
git push origin v0.1.0
```

O que acontece, em ordem:

1. `gates` → `version` (tag × manifestos) → `matrix` (lê `RELEASE_PLATFORMS`).
2. **O run PAUSA.** O job `build` está no Environment `release`, que exige **revisor humano (o dono)**. Aparece como *"Review deployments"* na aba Actions. **Não é bug — é a trava funcionando, e ninguém além do dono aprova.**
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
- **O `publish` já quebrou por causa de um glob.** Ele globava `artifacts/*.dmg` mesmo num release só de Linux — o glob não expandia e o `gh` morria com "no such file". Hoje a lista sai de um `find`, e um release **sem instalador nenhum falha alto**. Se mexer nesse passo, teste os três casos: só Linux, Linux+macOS, e nenhum instalador.
- **O input `dry_run` é decorativo.** Está declarado e **não é usado em lugar nenhum**. Quem decide tudo é `startsWith(github.ref, 'refs/tags/v')`. Não confie nele.
- **Ensaio só sai da `main` ou de tag.** O Environment `release` recusa branch arbitrária — o job nem começa, zero steps. É o custo consciente da proteção.
- **Nunca gere a chave de assinatura de update no CI.** ADR aceita: o app verifica com a chave pública embutida, então canal e chave são defesas independentes; pôr a chave no CI colapsa as duas numa só. E o updater **não tem revogação**.

## Fechar

- Publicar o rascunho no GitHub (o dono decide o momento).
- **Publicar na AUR** (`packaging/aur/PKGBUILD`), se a conta já existir:
  1. `pkgver` = a versão publicada.
  2. Substituir `__SHA256_DEB__` pelo **sha256 real do `.deb` publicado** (`sha256sum` do arquivo baixado do release). O placeholder existe para **falhar alto**: `sha256sums=('SKIP')` desliga a verificação, e num pacote `-bin` o checksum é a única coisa que amarra o que o usuário instala ao que nós publicamos.
  3. `makepkg --printsrcinfo > .SRCINFO` — a AUR **recusa o push sem isso**, e ele tem que ser regerado sempre que o PKGBUILD mudar.
  4. Commit e push em `ssh://aur@aur.archlinux.org/tyba-bin.git`.

  **Feito localmente, de propósito.** Automatizar exigiria guardar uma chave SSH da AUR num secret — e quem a tiver publica um PKGBUILD apontando para onde quiser. Mesma regra da chave do repo apt e da chave de update: publicação sensível não mora no CI.
- **Gravar a versão publicada em `tyba-site/src/lib/release.ts` (`LAST_KNOWN_RELEASE`)** — versão + plataformas que de fato saíram. **Este passo não é cosmético.** O site lê a Releases API sem token, e a Vercel roda em IP compartilhado: sob rate limit o fetch falha, e sem essa memória a página conclui *"nenhum binário publicado"* — ou seja, no pico de tráfego ela diz ao usuário que o download não existe. A regra: **o site pode não saber que saiu versão nova; nunca pode dizer que não existe binário quando existe.**
- Conferir que `tyba.dev/{locale}/download` já mostra a versão (a página lê a Releases API com cache de 1h — pode levar até uma hora, ou force um redeploy).
- Conferir que o changelog do site tem a versão publicada, **sem o selo de pré-release** (ele some sozinho quando o release existe).

# TODO

O que ficou de fora do lançamento **de propósito**, com o porquê. Não é lista de desejos: cada item aqui foi cortado por uma razão que continua valendo até alguém a derrubar.

Fases e itens já entregues vivem no [ROADMAP](ROADMAP.md).

## Avisar que saiu versão nova — **ENTREGUE** (v0.1.1)

A app consulta a Releases API do GitHub (cache de 6h, consulta fora do boot, sem chave nenhuma), compara semver estrito e mostra um toast **uma vez por versão**, com link pro changelog do site. Badge nos Ajustes pra quem fechou o toast.

**Sem opt-out, por decisão do dono e o argumento é bom**: o release pode ser um fix de segurança, e um toggle desligado deixaria o usuário vulnerável **sem saber que escolheu isso**. O que segura o abuso é o desenho, não a preferência.

Fica do item original:

- **What's new dentro da app** — hoje o toast leva pro changelog no navegador. Ver o que mudou **sem sair do TYBA** ainda não existe.

## Painel de CI — **ENTREGUE** (#149)

Runs, jobs e checks dentro do app. Fica de fora, com motivo:

- **Notificação quando o CI quebra com o painel fechado** — custaria poll de fundo competindo com os agentes por CPU e rede. Entra se o uso pedir.
- **Re-run / cancel** — é **escrita remota**, mesma classe de `push` e `gh pr create`. Precisa do gate de aprovação, não pode pegar carona num painel de leitura.
- **GitLab** — o core devolve `None` (não lista vazia: vazio diria "não há run"; `None` diz "não sei olhar aqui"). Não há repo GitLab pra exercitar o parser, e parser que ninguém roda é a mesma armadilha do teste de sandbox que passa com a jaula quebrada.

## Auto-update assinado (v0.2 — não antes)

**A chave privada de update é o secret mais perigoso do projeto — mais que o certificado da Apple.** Quem a tiver publica um "update" que a máquina de todo usuário **instala sozinha, sem clique**. O certificado da Apple, se vazar, deixa alguém assinar um app malicioso — mas o usuário ainda precisa baixar e abrir; a chave de update pula essa etapa. Num app que orquestra agentes com acesso ao código do usuário, é a superfície mais valiosa que existe pra atacar.

Introduzir isso junto com o primeiro release, na correria, é exatamente como se erra. Entra na v0.2, com a chave gerada e guardada com calma.

**E no Linux ele mal se aplica**: `.deb` e `.rpm` pertencem ao gerenciador de pacotes — a app não pode (nem deve) se sobrescrever por cima do `apt`. Só o AppImage se auto-atualiza. Ou seja, mesmo com auto-update, a maioria dos usuários Linux receberia... uma notificação. Que é o item 1.

Regra: **notificar sempre; auto-instalar só onde faz sentido** (`.dmg` e AppImage), nunca por cima do gerenciador de pacotes.

## Distribuição

- **Certificado Developer ID da Apple** — único bloqueador do lançamento macOS. O pipeline já lê os secrets certos e **recusa publicar** artefato não assinado (`codesign` + `spctl`): build ad-hoc é rejeitado pelo Gatekeeper e o usuário vê "app danificado". Passo a passo no cofre (`tyba/release/assinatura-e-distribuicao`).
- **AUR — cortada por decisão do dono.** O `packaging/aur/PKGBUILD` fica no repo, mas não vai pra AUR: o AppImage já cobre quem não usa `.deb`/`.rpm`. O custo aceito é conhecido — o usuário de Arch **não** recebe update pelo `yay` junto com o resto do sistema, e baixa o AppImage à mão a cada versão. Se isso doer no uso real, o PKGBUILD está pronto e só falta a conta.
- **Windows — shippou com jaula Camada A parcial** (`feat(windows)`, v0.1.2). O terminal funciona (ConPTY próprio, picker cmd/PowerShell/WSL) e a sessão de **agente** roda enjaulada: token `WRITE_RESTRICTED` + IL Low, ConPTY sob o token, gate de aprovação por named pipe herdado, env por allowlist, deny dos segredos nomeados, fail-closed. **Decisão real: token restrito, NÃO AppContainer** — ADR `tyba/decisions/2026-07-14-windows-token-restrito-nao-appcontainer` (a spec v1 era Camada A **+** B juntas; o release saiu com A). Cortes que o release não registrava e agora registra:
  - **Smoke no app real nunca rodou** — o binário Windows já sai no release, mas nenhum agente de verdade (claude/node) foi exercido enjaulado ponta a ponta no produto com console real (isTTY + render). Provado só separado: efeito no filesystem headless + spike de isTTY. Modo de falha é fail-closed, mas o caminho é não-exercido. **Barato e alto valor — precisa da máquina Windows.**
  - **Camada B fora do produto** — usuário dedicado + rede por WFP (por SID) + read default-deny estão **inteiros medidos em spike** (par positivo, máquina real com UAC), mas nada virou produto. Enquanto isso, o agente no Windows **alcança a rede livremente** (loopback/RFC1918/internet) e **lê a maior parte do filesystem** (só segredos nomeados são negados) — materialmente mais fraco que mac/Linux, numa plataforma que a tese de segurança acabou de abrir.
  - **Shim de git fora do produto** — mecanismo de relay validado (`gitshim`), grill de desenho pendente (binário vs junction; rotear todo git vs só escrita). Com a receita da jaula `git.exe` **já inicia** (some o `0xc0000142`), então é lacuna de **controle** (git de escrita deveria rodar no core), não de capacidade.
  - **Certificado de assinatura (SmartScreen)** — o `.exe`/MSI saem unsigned; sem cert o Windows avisa "app não reconhecido". Mesma classe do bloqueador da Apple.
- **Repositório apt/dnf próprio (à la Warp)** — o Warp hospeda `releases.warp.dev/linux/{deb,rpm}` assinado com GPG, e é isso que faz `apt upgrade` atualizar o app junto com o sistema. Resolveria de uma vez a AUR **e** o auto-update no Linux. **Mas um repo assinado é auto-update pela porta dos fundos**: quem tiver a chave GPG publica um pacote que a máquina instala no próximo `apt upgrade` — e com `unattended-upgrades`, sem clique nenhum. É praticamente a mesma superfície da chave de update que esta lista chama de *o secret mais perigoso do projeto*. Vale fazer, com o mesmo cuidado: chave gerada offline, **nunca no CI**, guardada no Environment `release`.
- **Flatpak** — fora da v0.1. O TYBA **é** um sandbox, e bwrap aninhado dentro do bwrap do Flatpak não sobe. O caminho é `flatpak-spawn --host` (precedente Ptyxis/Black Box no Flathub): PTY, socket de hook e a jaula toda atravessando a fronteira do container. É projeto próprio, não um manifesto — e os formatos nativos (.deb/.rpm/AppImage) cobrem o público de dev da v0.1.

## Segurança

- **Sandbox Linux em kernel sem user namespace** — Ubuntu 24.04 (AppArmor restritivo) e kernels hardened. Hoje: sessão de agente recusada fail-closed e op de git com filtro recusada, ambas com mensagem acionável. Falta decidir se vale um card de escalada na UI ensinando a habilitar, em vez de só o erro.
- **Card de escalada quando o sandbox nega** — hoje a falha é seca (deny do SO é indistinguível do erro real; o Codex usa heurística de string e assume falso-positivo). Os paths de toolchain já vêm liberados e o dono acrescenta o que faltar em `~/.tyba/config.toml`. Melhoria posterior, quando a lista de paths estabilizar com uso real.
- **Runner Custom** — bloqueado **por desenho**: binário arbitrário não tem hooks, logo não tem `PreToolUse`, logo **não tem gate de aprovação**. Só faz sentido quando a contenção vier inteira do SO e o inbox aprovar escaladas.
- **Testes automáticos pós-resolução de conflito** — o agente resolve e para antes do commit; não há verificação de que o resultado compila.

## Produto

- Explorer fase 2 restante: criar branch pela UI; badge ahead/behind (precisa de cache).
- Abort da operação de conflito pela UI; merge 3-way visual.
- Seletor de base/branch no painel de diff (three-dot/merge-base) — grill pendente.
- LSP para contexto de agente (tsserver/rust-analyzer).
- Shell integration própria (OSC 133) com blocos de comando — spec e ADR já aceitos.
- OSC 52 com confirmação, sanitização OSC 8.

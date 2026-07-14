# TODO

O que ficou de fora do lançamento **de propósito**, com o porquê. Não é lista de desejos: cada item aqui foi cortado por uma razão que continua valendo até alguém a derrubar.

Fases e itens já entregues vivem no [ROADMAP](ROADMAP.md).

## Avisar que saiu versão nova (v0.1.x)

Hoje **não existe nada**: nenhum `tauri-plugin-updater`, nenhuma checagem de versão, nenhum "what's new" — a app nem expõe a própria versão pra UI. Lançar assim significa não ter como avisar os primeiros usuários que saiu o fix do bug que eles reportaram.

São três coisas diferentes e a ordem importa:

1. **Notificação de versão nova** — consultar a Releases API do GitHub, comparar com a versão da app, mostrar um toast com link. **Não precisa de chave nenhuma.** Resolve o essencial: o usuário fica sabendo e decide.
2. **What's new** — `CHANGELOG.md` + as notas do release (o pipeline já usa `--generate-notes`, que monta a lista a partir dos títulos de PR; os títulos já nascem descritivos).
3. **Auto-update de verdade** — ver abaixo, fica pra v0.2.

Pré-requisito comum: expor a versão da app ao front (não existe hoje) e cachear a consulta pra não pesar no boot.

## Auto-update assinado (v0.2 — não antes)

**A chave privada de update é o secret mais perigoso do projeto — mais que o certificado da Apple.** Quem a tiver publica um "update" que a máquina de todo usuário **instala sozinha, sem clique**. O certificado da Apple, se vazar, deixa alguém assinar um app malicioso — mas o usuário ainda precisa baixar e abrir; a chave de update pula essa etapa. Num app que orquestra agentes com acesso ao código do usuário, é a superfície mais valiosa que existe pra atacar.

Introduzir isso junto com o primeiro release, na correria, é exatamente como se erra. Entra na v0.2, com a chave gerada e guardada com calma.

**E no Linux ele mal se aplica**: `.deb` e `.rpm` pertencem ao gerenciador de pacotes — a app não pode (nem deve) se sobrescrever por cima do `apt`. Só o AppImage se auto-atualiza. Ou seja, mesmo com auto-update, a maioria dos usuários Linux receberia... uma notificação. Que é o item 1.

Regra: **notificar sempre; auto-instalar só onde faz sentido** (`.dmg` e AppImage), nunca por cima do gerenciador de pacotes.

## Distribuição

- **Certificado Developer ID da Apple** — único bloqueador do lançamento macOS. O pipeline já lê os secrets certos e **recusa publicar** artefato não assinado (`codesign` + `spctl`): build ad-hoc é rejeitado pelo Gatekeeper e o usuário vê "app danificado". Passo a passo no cofre (`tyba/release/assinatura-e-distribuicao`).
- **AUR — cortada por decisão do dono.** O `packaging/aur/PKGBUILD` fica no repo, mas não vai pra AUR: o AppImage já cobre quem não usa `.deb`/`.rpm`. O custo aceito é conhecido — o usuário de Arch **não** recebe update pelo `yay` junto com o resto do sistema, e baixa o AppImage à mão a cada versão. Se isso doer no uso real, o PKGBUILD está pronto e só falta a conta.
- **Windows** — a compilação é trivial (Tauri roda, `portable-pty` fala ConPTY, `default_shell()` já trata `COMSPEC`); a **jaula** não existe. Sessão de shell (PowerShell/cmd/WSL) funciona sem jaula — só a de **agente** é recusada. Caminho decidido: **AppContainer nativo, não WSL2** — ADR no cofre (`tyba/decisions/2026-07-14-windows-appcontainer-nao-wsl`). Falta ainda o certificado de assinatura (SmartScreen).
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

//! O comando que o `sshd` executa numa SSH Session integrada.
//!
//! O script local de integração (`tyba-bash-rc.sh`, `tyba-zsh-rc.sh`) viaja
//! dentro do próprio comando do `ssh`, em base64, é materializado numa pasta
//! privada do servidor só para o shell ler na partida, e **a pasta é apagada
//! pelo próprio rc** (regra 9). Depois que a sessão sobe, nada do TYBA fica no
//! servidor.

use base64::Engine;

use super::tmux;

/// O shell de login do servidor, como o `$SHELL` dele o declara.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteShell {
    Bash,
    Zsh,
    /// Qualquer outro (fish, ksh, sh do BusyBox) — e também o que não deu para
    /// detectar. Abre sessão comum, com o motivo (regra 8).
    Unsupported(String),
}

impl RemoteShell {
    /// Classifica pelo caminho que o `$SHELL` do servidor devolveu.
    pub fn from_path(shell: &str) -> Self {
        let name = shell
            .trim()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .trim_end_matches(".exe");
        match name {
            "bash" => RemoteShell::Bash,
            "zsh" => RemoteShell::Zsh,
            "" => RemoteShell::Unsupported("desconhecido".into()),
            other => RemoteShell::Unsupported(other.to_string()),
        }
    }

    pub fn integrable(&self) -> bool {
        matches!(self, RemoteShell::Bash | RemoteShell::Zsh)
    }

    /// O nome que vai no motivo do evento de integração.
    pub fn label(&self) -> &str {
        match self {
            RemoteShell::Bash => "bash",
            RemoteShell::Zsh => "zsh",
            RemoteShell::Unsupported(name) => name,
        }
    }
}

const CTL_MARKER_KEY: &str = "tyba-ctl=";

/// O marco que anuncia a troca para o protocolo de controle (regra 1). Mesmo
/// canal e mesmo nonce do marco de login, para que um marco de outro Cano — num
/// scrollback, num log — não troque o transporte desta sessão.
pub fn control_marker(nonce: &str) -> String {
    format!("\x1b]633;P;{CTL_MARKER_KEY}{nonce}\x07")
}

/// A pasta some assim que o rc é lido. No bash isso acontece no TOPO do próprio
/// arquivo, e é de propósito: o POSIX garante que o arquivo já aberto sobrevive
/// ao `unlink`, então o shell termina de ler o que abriu. Apagar no fim deixaria
/// rastro toda vez que o `.bashrc` do dono fizesse `return`.
const REMOTE_PROLOGUE: &str = "\
# --- TYBA: prólogo remoto ---------------------------------------------------\n\
# A pasta é apagada AQUI, com o arquivo ainda aberto pelo shell: nada do TYBA\n\
# pode sobrar no servidor depois que a sessão sobe (regra 9 da entrega).\n\
if [ -n \"${TYBA_RC_DIR:-}\" ]; then\n\
  rm -rf \"$TYBA_RC_DIR\"\n\
  unset TYBA_RC_DIR\n\
fi\n";

/// O mesmo, para o fim do `.zshrc` remoto: ali a pasta só pode sumir depois que
/// o último arquivo da cadeia (`.zshenv`, `.zprofile`, `.zshrc`) foi lido.
const REMOTE_EPILOGUE: &str = "\n\
# --- TYBA: epílogo remoto ---------------------------------------------------\n\
if [ -n \"${TYBA_RC_DIR:-}\" ]; then\n\
  rm -rf \"$TYBA_RC_DIR\"\n\
  unset TYBA_RC_DIR\n\
fi\n";

pub const BASH_RC_NAME: &str = "tyba-bash-rc.sh";

/// Os arquivos que o rc remoto precisa, na ordem em que o shell os lê.
///
/// São os MESMOS scripts da integração local (`include_str!` em
/// `session/mod.rs`) mais o prólogo que apaga a pasta — duplicar o conteúdo
/// deixaria as duas integrações divergirem em silêncio.
pub fn rc_files(shell: &RemoteShell) -> Vec<(&'static str, String)> {
    match shell {
        RemoteShell::Bash => vec![(
            BASH_RC_NAME,
            format!("{REMOTE_PROLOGUE}{}", crate::session::TYBA_BASH_RC),
        )],
        RemoteShell::Zsh => vec![
            (".zshenv", crate::session::zsh_chain(".zshenv")),
            (".zprofile", crate::session::zsh_chain(".zprofile")),
            (
                ".zshrc",
                format!(
                    "{}\n{}\n{REMOTE_EPILOGUE}ZDOTDIR=\"$TYBA_USER_ZDOTDIR\"\n",
                    crate::session::zsh_chain(".zshrc"),
                    crate::session::TYBA_ZSH_RC
                ),
            ),
        ],
        RemoteShell::Unsupported(_) => Vec::new(),
    }
}

/// O comando do pane: o shell do dono, carregando o rc do TYBA.
///
/// `$d` já está expandido pelo `sh` de fora quando tmux recebe a string; as
/// aspas escapadas são as que o lexer do tmux vai consumir.
fn pane_command(shell: &RemoteShell, prompt_mode: bool) -> String {
    let prompt = if prompt_mode {
        " TYBA_PROMPT_MODE=1 SPACESHIP_PROMPT_ASYNC=false"
    } else {
        ""
    };
    match shell {
        RemoteShell::Bash => format!(
            "exec env -u TMUX TYBA_RC_DIR=\\\"$d\\\" TYBA_LOGIN_SHELL=1{prompt} \
             bash --rcfile \\\"$d/{BASH_RC_NAME}\\\" -i"
        ),
        RemoteShell::Zsh => format!(
            "exec env -u TMUX TYBA_RC_DIR=\\\"$d\\\" ZDOTDIR=\\\"$d\\\" \
             TYBA_USER_ZDOTDIR=\\\"${{ZDOTDIR:-$HOME}}\\\"{prompt} zsh -l -i"
        ),
        RemoteShell::Unsupported(_) => String::new(),
    }
}

/// Idem, sem tmux: o shell integrado nasce direto, sem persistência (regra 13).
/// Sem o embrulho de aspas do tmux, porque aqui quem executa é o `sh` de fora.
fn direct_command(shell: &RemoteShell, prompt_mode: bool) -> String {
    pane_command(shell, prompt_mode).replace("\\\"", "\"")
}

/// O sufixo da pasta privada do servidor, sorteado aqui e embutido no comando.
///
/// O TYBA precisa saber o nome ANTES de a pasta existir — é o que permite armar
/// a armadilha de limpeza antes da criação (regra 9).
///
/// Armadilha: este token NÃO pode ser o nonce dos marcos. O nome da pasta é
/// legível por qualquer usuário local do servidor (`ls /run/user/0`), e o nonce
/// é justamente o que autentica os marcos OSC — reusá-lo entregaria de graça o
/// segredo que permite forjar marco de login ou de controle. Mesma fonte de
/// aleatoriedade (UUID v4), segredo separado.
fn dir_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn b64(raw: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
}

/// O comando remoto da sessão.
///
/// `integrate = false` (chave do Host desligada) ou shell não suportado devolve
/// exatamente o comando de antes desta entrega — sessão comum, marco de login
/// no lugar de sempre.
pub fn remote_command(shell: RemoteShell, nonce: &str, tmux_name: &str, integrate: bool) -> String {
    remote_command_with(shell, nonce, tmux_name, integrate, true)
}

/// O mesmo, com o modo prompt do TYBA escolhido por quem chama.
///
/// Emenda de contrato — o `remote_command` da spec não carrega o modo prompt, e
/// ele não pode ser fixo: quem desligou `pref.promptMode` desligou para a
/// sessão remota também. Sem isto o shell remoto nasceria com o `PS1` fora da
/// tela contra a preferência do dono.
pub fn remote_command_with(
    shell: RemoteShell,
    nonce: &str,
    tmux_name: &str,
    integrate: bool,
    prompt_mode: bool,
) -> String {
    if !integrate || !shell.integrable() {
        return tmux::wrap_command_with_nonce(tmux_name, nonce);
    }
    let files = rc_files(&shell)
        .into_iter()
        .map(|(name, body)| format!("{name}:{}", b64(&body)))
        .collect::<Vec<_>>()
        .join(" ");
    let pane = pane_command(&shell, prompt_mode);
    let direct = direct_command(&shell, prompt_mode);
    let script = format!(
        // Marco de login primeiro: vale para TODOS os ramos abaixo, inclusive o
        // que não integra nada.
        //
        // A ORDEM aqui é a regra 9 inteira: o nome da pasta é sorteado pelo TYBA
        // e viaja pronto, então a armadilha pode ser armada ANTES de a pasta
        // existir. O contrário — criar e só então armar — deixa uma janela em
        // que o SIGHUP da queda do `ssh` mata o `sh` pela ação padrão com a
        // pasta já montada, e ela fica no servidor para sempre. `rm -rf` num
        // caminho que ainda não existe é inócuo. Função, e não comando entre
        // aspas simples, porque aspas simples fechariam o envelope do `sh -c` no
        // meio. O caminho feliz termina em `exec`, que troca a imagem do
        // processo — saída normal não dispara armadilha nenhuma, e por isso não
        // há `EXIT` aqui.
        //
        // `(umask 077; mkdir)` no lugar de `mkdir` + `chmod 700`: fecha também a
        // janela de MODO entre criar e ajustar, e tira o `chmod` da lista do que
        // o PATH do servidor precisa ter. `mkdir` falha se o caminho já existir,
        // e é essa falha que guarda contra symlink ou pré-criação de outro
        // usuário — quem falha aqui degrada para sessão comum, nunca tenta outro
        // nome.
        "{login} \
         b=; printf %s Zm9v | base64 -d >/dev/null 2>&1 && b=\"base64 -d\"; \
         [ -z \"$b\" ] && printf %s Zm9v | base64 -D >/dev/null 2>&1 && b=\"base64 -D\"; \
         {attach_branch} \
         d=; [ -n \"$b\" ] && d=\"${{XDG_RUNTIME_DIR:-/tmp}}/tyba-{token}\"; \
         tyba_wipe() {{ [ -n \"$d\" ] && rm -rf \"$d\" 2>/dev/null; }}; \
         trap tyba_wipe HUP TERM INT; \
         [ -n \"$d\" ] && {{ (umask 077; mkdir \"$d\") 2>/dev/null || d=; }}; \
         for f in {files}; do \
           [ -n \"$d\" ] || break; \
           printf %s \"${{f#*:}}\" | $b > \"$d/${{f%%:*}}\" 2>/dev/null || {{ rm -rf \"$d\"; d=; }}; \
         done; \
         {tmux_branch} \
         [ -n \"$d\" ] && {direct}; \
         exec \"${{SHELL:-/bin/sh}}\" -l",
        login = tmux::login_marker_printf(nonce),
        attach_branch = attach_branch(tmux_name, nonce),
        tmux_branch = tmux_branch(tmux_name, nonce, &pane),
        token = dir_token(),
    );
    tmux::sh_c(&script)
}

/// O ramo de REATAR, antes de qualquer arquivo tocar o servidor.
///
/// Medido na VPS do dono: 12 pastas `tyba-*` em `/run/user/0`, uma por
/// reatar. A causa é o `-A`: com a sessão já viva ele **anexa** e o comando do
/// pane nunca roda, então ninguém lê o rc e o `rm -rf` que mora dentro dele
/// jamais executa (regra 9). Quem anexa não tem o que materializar — o pane já
/// está de pé com o rc que ele leu no dia em que nasceu.
///
/// Armadilhas:
///
/// - **A corrida entre perguntar e anexar.** A sessão pode morrer no meio; aí o
///   `attach-session` falha, o `ssh` cai e o Cano religa — na tentativa
///   seguinte o `has-session` responde "não existe" e o caminho de criar roda
///   inteiro. Custa uma reconexão, e é de propósito: `new-session -A` aqui
///   criaria uma sessão SEM o comando do pane, ou seja, um shell sem o rc do
///   TYBA se passando por sessão integrada.
/// - **O marco de controle sai antes do `exec`.** Se o `attach-session` falhar,
///   o erro do tmux chega ao core já em modo de controle: vira uma linha não
///   reconhecida, logada uma vez, e a sessão morre logo em seguida pelo lado do
///   Cano (regras 6 e 7).
/// - **`$b` vazio** (servidor sem `base64`) nunca chega aqui: ali o rc não pôde
///   ser materializado nem na criação, a sessão viva é comum, e anunciar o
///   protocolo agora trocaria o transporte de uma sessão que nunca falou dele.
/// - **Host sem tmux** (regra 13) não passa do `command -v`: não há sessão para
///   anexar, e o shell integrado direto nasce lá embaixo, com rc e com a
///   pasta que ele mesmo apaga.
/// - A armadilha do `trap` fica onde estava: ela protege a janela entre o
///   criação da pasta e o `exec`, que este ramo nem abre.
/// - **As opções do pane (`status off`, `prefix None`, `history-limit`) não são
///   repetidas aqui**: elas vivem na SESSÃO, e a sessão que se anexa é a que
///   este mesmo código criou com elas.
///
/// Medido contra o tmux 3.7c local em 2026-09-18: `has-session` devolve 0/1,
/// `-C attach-session` fala o protocolo e entrega o `%output` do pane que já
/// estava lá, e anexar numa sessão que sumiu devolve `%error` com `can't find
/// session` e sai — que é o desfecho ordenado da corrida acima.
fn attach_branch(name: &str, nonce: &str) -> String {
    format!(
        "command -v tmux >/dev/null 2>&1 && [ -n \"$b\" ] && \
         tmux has-session -t {name} 2>/dev/null && {{ \
           stty -echo 2>/dev/null; \
           printf \"\\033]633;P;tyba-ctl=%s\\007\" {nonce}; \
           exec tmux -C attach-session -t {name}; \
         }};"
    )
}

/// O ramo do tmux: integrado vira `tmux -C` e só ele imprime o marco de
/// controle. Sem rc materializado o host cai no `new-session` de sempre — o
/// marco NÃO sai, e o core continua lendo byte cru, que é o que ele é.
///
/// O `stty -echo` mora nos dois ramos de controle e só neles: o terminal do
/// CLIENTE de controle não tem dono humano — o que se escreve ali são comandos
/// do tmux —, enquanto o terminal do shell comum é do dono e quem manda no eco
/// dele é o próprio shell. A fase crua (banner, senha, confirmação da chave do
/// host) não é tocada: ela acontece ANTES de o `sshd` executar este comando, e
/// é por isso que o `-t` continua onde estava.
///
/// Medido contra o tmux 3.7c local em 2026-09-18, porque as duas pontas dessa
/// decisão eram palpite: (1) o tmux em modo de controle NÃO religa o eco do
/// próprio terminal — com o `stty` o comando escrito não volta, sem ele volta;
/// (2) o termios do cliente NÃO vaza para o pane — `stty -a` de dentro do pane
/// sai idêntico com e sem o `stty -echo` aqui, e com `echo` ligado. Se vazasse,
/// o dono digitaria num `cat` sem ver o que digitou.
fn tmux_branch(name: &str, nonce: &str, pane: &str) -> String {
    format!(
        "command -v tmux >/dev/null 2>&1 && {{ \
           [ -n \"$d\" ] && {{ \
             stty -echo 2>/dev/null; \
             printf \"\\033]633;P;tyba-ctl=%s\\007\" {nonce}; \
             exec tmux -C new-session -A -s {name} \"{pane}\" \\; \
               set-option -t {name} status off \\; \
               set-option -t {name} prefix None \\; \
               set-option -t {name} history-limit 5000; \
           }}; \
           exec tmux new-session -A -s {name} \"exec env -u TMUX \\\"${{SHELL:-/bin/sh}}\\\" -l\" \\; \
             set-option -t {name} status off \\; \
             set-option -t {name} prefix None \\; \
             set-option -t {name} history-limit 5000; \
         }};"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "0123456789abcdef0123456789abcdef";
    const NAME: &str = "tyba-a3f-9f3a";

    fn integrated(shell: RemoteShell) -> String {
        remote_command(shell, NONCE, NAME, true)
    }

    #[test]
    fn o_shell_remoto_sai_do_caminho_do_shell() {
        assert_eq!(RemoteShell::from_path("/bin/bash"), RemoteShell::Bash);
        assert_eq!(RemoteShell::from_path("/usr/bin/zsh\n"), RemoteShell::Zsh);
        assert_eq!(
            RemoteShell::from_path("/usr/bin/fish"),
            RemoteShell::Unsupported("fish".into())
        );
        assert_eq!(
            RemoteShell::from_path(""),
            RemoteShell::Unsupported("desconhecido".into()),
            "detecção que falhou é sessão comum, não palpite de bash"
        );
    }

    #[test]
    fn o_marco_de_login_sai_antes_do_marco_de_controle_e_do_tmux() {
        let cmd = integrated(RemoteShell::Bash);
        let login = cmd
            .find("tyba-ssh-login=")
            .expect("o marco de login continua sendo o primeiro");
        let ctl = cmd.find("tyba-ctl=").expect("o marco de controle");
        let tmux = cmd
            .find("exec tmux -C new-session")
            .expect("o tmux de controle");
        assert!(login < ctl, "{cmd}");
        assert!(ctl < tmux, "{cmd}");
    }

    /// O eco só é desligado onde o terminal remoto serve ao PROTOCOLO. No shell
    /// comum e no shell direto (host sem tmux, regra 13) o terminal é do dono, e
    /// quem manda no eco dele é o shell.
    #[test]
    fn o_eco_so_morre_nos_ramos_do_cliente_de_controle() {
        let cmd = integrated(RemoteShell::Bash);
        for exec in ["exec tmux -C attach-session", "exec tmux -C new-session"] {
            let stty = cmd[..cmd.find(exec).expect(exec)].rfind("stty -echo");
            let anterior = cmd[..cmd.find(exec).unwrap()]
                .rfind("printf \"\\033]633;P;tyba-ctl=")
                .expect("o marco de controle");
            assert!(
                stty.is_some_and(|at| at < anterior),
                "{exec}: o eco morre antes do marco, senão o próprio marco volta \
                 ecoado: {cmd}"
            );
        }
        assert_eq!(
            cmd.matches("stty -echo").count(),
            2,
            "só os dois ramos de controle: {cmd}"
        );
        let direto = cmd
            .find("exec env -u TMUX TYBA_RC_DIR=\"$d\"")
            .expect("direto");
        assert!(
            cmd[direto..].find("stty -echo").is_none(),
            "o shell direto do host sem tmux é do dono: {cmd}"
        );
        assert!(
            !tmux::wrap_command_with_nonce(NAME, NONCE).contains("stty"),
            "sessão comum não muda de terminal por causa desta entrega"
        );
    }

    #[test]
    fn a_chave_desligada_devolve_o_comando_de_sempre() {
        let plain = remote_command(RemoteShell::Bash, NONCE, NAME, false);
        assert_eq!(plain, tmux::wrap_command_with_nonce(NAME, NONCE));
        assert!(!plain.contains("tyba-ctl="), "{plain}");
    }

    #[test]
    fn shell_nao_suportado_nunca_vira_sessao_integrada() {
        let cmd = remote_command(RemoteShell::Unsupported("fish".into()), NONCE, NAME, true);
        assert_eq!(cmd, tmux::wrap_command_with_nonce(NAME, NONCE));
    }

    #[test]
    fn o_rc_remoto_e_o_mesmo_script_da_integracao_local() {
        let bash = rc_files(&RemoteShell::Bash);
        assert_eq!(bash.len(), 1);
        assert!(
            bash[0].1.contains("__tyba_preexec"),
            "o corpo tem de ser o rc local, não uma cópia"
        );
        assert!(bash[0].1.contains(crate::session::TYBA_BASH_RC));

        let zsh = rc_files(&RemoteShell::Zsh);
        assert_eq!(
            zsh.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            vec![".zshenv", ".zprofile", ".zshrc"],
            "a cadeia do zsh é lida nesta ordem; o .zlogin do dono volta a valer \
             quando o ZDOTDIR é restaurado no fim do .zshrc"
        );
        assert!(zsh[2].1.contains(crate::session::TYBA_ZSH_RC));
    }

    #[test]
    fn o_rc_apaga_a_propria_pasta() {
        for shell in [RemoteShell::Bash, RemoteShell::Zsh] {
            let files = rc_files(&shell);
            let apaga = files
                .iter()
                .any(|(_, body)| body.contains("rm -rf \"$TYBA_RC_DIR\""));
            assert!(
                apaga,
                "{}: nada do TYBA pode ficar no servidor",
                shell.label()
            );
        }
    }

    /// O script gerado, executado de verdade por um `sh`, contra um PATH
    /// montado à mão. É o único teste local que prova a regra 9 sem servidor:
    /// o rc chega, o shell o lê, e a pasta some.
    #[cfg(unix)]
    mod no_servidor_de_mentira {
        use super::*;
        use std::process::{Command, Stdio};

        /// PATH mínimo, com os utilitários que o script usa e **sem tmux**: o
        /// host sem tmux é justamente o ramo que abre integrado e sem
        /// persistência (regra 13).
        pub(super) fn fake_path(
            dir: &std::path::Path,
            tools: &[&str],
        ) -> Option<std::path::PathBuf> {
            let bin = dir.join("bin");
            std::fs::create_dir_all(&bin).ok()?;
            for tool in tools {
                let found = Command::new("/bin/sh")
                    .arg("-c")
                    .arg(format!("command -v {tool}"))
                    .output()
                    .ok()?;
                let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
                if !found.status.success() || path.is_empty() {
                    return None;
                }
                std::os::unix::fs::symlink(&path, bin.join(tool)).ok()?;
            }
            Some(bin)
        }

        /// Um `tmux` de mentira no PATH falso, com a semântica do `-A` que o
        /// defeito da regra 9 expôs: quando a sessão JÁ existe, `new-session -A`
        /// **anexa** e o comando do pane é ignorado — ninguém lê o rc, e quem o
        /// materializou deixa a pasta no servidor.
        ///
        /// `existe = true` é o reatar; `false` é a sessão nascendo.
        fn fake_tmux(bin: &std::path::Path, existe: bool, log: &std::path::Path) {
            let tmux = bin.join("tmux");
            std::fs::write(
                &tmux,
                format!(
                    "#!/bin/sh\n\
                     log() {{ printf '%s\\n' \"$1\" >> \"{log}\"; }}\n\
                     for a in \"$@\"; do\n\
                     case \"$a\" in\n\
                     has-session) exit {has} ;;\n\
                     attach-session) log attach; exit 0 ;;\n\
                     esac\n\
                     done\n\
                     [ {has} = 0 ] && {{ log attach; exit 0; }}\n\
                     while [ $# -gt 0 ]; do\n\
                     if [ \"$1\" = -s ]; then shift 2; log new; exec sh -c \"$1\"; fi\n\
                     shift\n\
                     done\n\
                     exit 0\n",
                    log = log.display(),
                    has = if existe { 0 } else { 1 },
                ),
            )
            .unwrap();
            std::fs::set_permissions(&tmux, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
        }

        /// Regra 9 no reatar: a sessão do tmux já existe, então nenhum shell
        /// novo nasce e ninguém lê o rc. Medido na VPS do dono: 12 pastas
        /// `tyba-*` em `/run/user/0`, uma por reatar.
        #[test]
        fn reatar_nao_deixa_pasta_no_servidor() {
            let home = tempfile::tempdir().unwrap();
            let run = tempfile::tempdir().unwrap();
            let Some(bin) = fake_path(
                home.path(),
                &["sh", "bash", "base64", "mkdir", "env", "rm", "stty"],
            ) else {
                return; // máquina sem um dos utilitários: nada a provar aqui
            };
            let log = home.path().join("tmux.log");
            fake_tmux(&bin, true, &log);

            let cmd = integrated(RemoteShell::Bash);
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            let out = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .env_clear()
                .env("PATH", &bin)
                .env("HOME", home.path())
                .env("XDG_RUNTIME_DIR", run.path())
                .env("SHELL", bin.join("bash"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .unwrap();

            assert_eq!(
                std::fs::read_to_string(&log).unwrap_or_default().trim(),
                "attach",
                "o ramo do reatar é o de anexar, não o de criar"
            );
            let leftovers: Vec<String> = std::fs::read_dir(run.path())
                .unwrap()
                .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
                .collect();
            assert!(
                leftovers.is_empty(),
                "quem anexa não materializa rc nenhum: o pane já existe e ninguém \
                 leria o arquivo; sobrou {leftovers:?}"
            );
            assert!(
                String::from_utf8_lossy(&out.stdout).contains("tyba-ctl="),
                "anexar continua anunciando a troca de protocolo (regra 1)"
            );
        }

        /// O outro lado do ramo: a sessão ainda não existe, o tmux cria, e é o
        /// pane que lê o rc e apaga a pasta. Guarda de regressão do caminho que
        /// o ramo de anexar não pode ter levado junto.
        #[test]
        fn criar_a_sessao_continua_materializando_o_rc_e_limpando() {
            let home = tempfile::tempdir().unwrap();
            let run = tempfile::tempdir().unwrap();
            let Some(bin) = fake_path(
                home.path(),
                &["sh", "bash", "base64", "mkdir", "env", "rm", "stty"],
            ) else {
                return; // máquina sem um dos utilitários: nada a provar aqui
            };
            let log = home.path().join("tmux.log");
            fake_tmux(&bin, false, &log);

            let cmd = integrated(RemoteShell::Bash);
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            let out = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .env_clear()
                .env("PATH", &bin)
                .env("HOME", home.path())
                .env("XDG_RUNTIME_DIR", run.path())
                .env("SHELL", bin.join("bash"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    child
                        .stdin
                        .as_mut()
                        .unwrap()
                        .write_all(probe_line(&RemoteShell::Bash).as_bytes())?;
                    child.wait_with_output()
                })
                .unwrap();

            assert_eq!(
                std::fs::read_to_string(&log).unwrap_or_default().trim(),
                "new",
                "sessão que não existe é criada, não anexada"
            );
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains("TYBA_MARK=1:\n"),
                "o pane criado pelo tmux lê o rc do TYBA e sai sem rastro da pasta; \
                 got: {stdout:?}"
            );
            let leftovers: Vec<String> = std::fs::read_dir(run.path())
                .unwrap()
                .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
                .collect();
            assert!(leftovers.is_empty(), "sobrou {leftovers:?}");
        }

        /// O que o shell integrado tem de responder quando o rc pegou. Cada
        /// shell reporta a própria marca de integração.
        fn probe_line(shell: &RemoteShell) -> &'static str {
            match shell {
                // O rc do zsh não marca variável nenhuma: o que prova que ele
                // foi lido é o hook existir.
                RemoteShell::Zsh => {
                    "printf 'TYBA_MARK=%s:%s\\n' \"${+functions[__tyba_osc7]}\" \"$TYBA_RC_DIR\"\nexit\n"
                }
                _ => {
                    "printf 'TYBA_MARK=%s:%s\\n' \"$TYBA_BASH_INTEGRATION\" \"$TYBA_RC_DIR\"\nexit\n"
                }
            }
        }

        #[test]
        fn o_rc_chega_ao_shell_remoto_e_a_pasta_some_depois() {
            roda(RemoteShell::Bash);
        }

        /// O zsh não tem `--rcfile`: a cadeia inteira passa pelo `ZDOTDIR`, e a
        /// pasta só pode sumir no FIM do `.zshrc` — o último arquivo que o TYBA
        /// põe no caminho antes de devolver o `ZDOTDIR` ao dono.
        #[test]
        fn o_rc_do_zsh_tambem_chega_e_tambem_some() {
            roda(RemoteShell::Zsh);
        }

        /// Um `mkdir` falso que trava por 30 s DENTRO da chamada, criando a
        /// pasta antes (`cria = true`) ou não criando nada (`cria = false`).
        ///
        /// É o que torna a janela sob teste **observável**, e o ponto é o
        /// `mkdir`: qualquer outro utilitário do prólogo está depois do `trap`
        /// nas duas ordens possíveis, e um teste preso ali ficaria verde com a
        /// ordem errada também. `dentro` só aparece com o script já parado lá —
        /// é por ele que o teste sabe onde o processo está antes do sinal, sem
        /// depender de temporização.
        ///
        /// O `umask 077` do script vale para o `mkdir` de verdade que este roda:
        /// umask é herdada, então o modo da pasta continua sendo o que a regra 9
        /// exige.
        fn mkdir_que_trava(bin: &std::path::Path, cria: bool, dentro: &std::path::Path) -> bool {
            let alvo = bin.join("mkdir");
            assert!(
                !alvo.exists(),
                "mkdir já está no PATH falso: escrever por cima seguiria o \
                 symlink e sobrescreveria o utilitário de verdade da máquina"
            );
            let Some(real) = onde("mkdir") else {
                return false;
            };
            let cria = if cria {
                format!("{real} \"$@\" || exit 1\n")
            } else {
                String::new()
            };
            std::fs::write(
                &alvo,
                format!(
                    "#!/bin/sh\n{cria}: > \"{dentro}\"\nsleep 30\n",
                    dentro = dentro.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&alvo, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            true
        }

        /// O caminho absoluto do utilitário de verdade, fora do PATH falso.
        fn onde(tool: &str) -> Option<String> {
            let found = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("command -v {tool}"))
                .output()
                .ok()?;
            let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
            (found.status.success() && !path.is_empty()).then_some(path)
        }

        /// Regra 9: a janela entre a criação da pasta e o `exec` também tem de
        /// fechar.
        ///
        /// Uma queda de conexão ali — SIGHUP, o cenário que a entrega anterior
        /// existe para tratar — mata o `sh` antes do `exec`, e sem a armadilha
        /// JÁ ARMADA a pasta fica no servidor para sempre. O sinal cai com o
        /// script parado dentro do próprio `mkdir`, com a pasta de pé.
        #[test]
        fn queda_no_meio_da_montagem_nao_deixa_pasta_no_servidor() {
            use std::os::unix::process::CommandExt;

            let home = tempfile::tempdir().unwrap();
            let run = tempfile::tempdir().unwrap();
            let Some(bin) = fake_path(home.path(), &["sh", "bash", "base64", "sleep", "env", "rm"])
            else {
                return; // máquina sem um dos utilitários: nada a provar aqui
            };
            let dentro = home.path().join("na-janela");
            if !mkdir_que_trava(&bin, true, &dentro) {
                return; // máquina sem mkdir: nada a provar aqui
            }

            let cmd = integrated(RemoteShell::Bash);
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .env_clear()
                .env("PATH", &bin)
                .env("HOME", home.path())
                .env("XDG_RUNTIME_DIR", run.path())
                .env("SHELL", bin.join("bash"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                // Grupo próprio: o sinal desta queda não pode alcançar quem roda
                // o teste.
                .process_group(0)
                .spawn()
                .unwrap();

            espera(&dentro, "o script nunca chegou à janela sob teste");
            let privadas = listagem(run.path());
            assert_eq!(
                privadas.len(),
                1,
                "a pasta privada tem de estar DE PÉ quando o sinal chega, senão o \
                 teste não observou a janela; achei {privadas:?}"
            );
            // Regra 9: 0700 desde o nascimento. O `umask 077` do script é o que
            // garante isso sem uma janela de modo entre criar e ajustar — um
            // `chmod` depois deixaria a pasta legível por um instante.
            let modo = std::os::unix::fs::PermissionsExt::mode(
                &std::fs::metadata(run.path().join(&privadas[0]))
                    .unwrap()
                    .permissions(),
            ) & 0o777;
            assert_eq!(modo, 0o700, "a pasta privada nasceu com modo {modo:o}");

            // SIGHUP no grupo é o que a queda do `ssh` entrega ao comando remoto.
            unsafe { libc::kill(-(child.id() as i32), libc::SIGHUP) };
            child.wait().unwrap();

            let leftovers = listagem(run.path());
            assert!(
                leftovers.is_empty(),
                "regra 9: a queda entre a criação da pasta e o exec tem de levar a \
                 pasta junto; sobrou {leftovers:?}"
            );
        }

        /// O simétrico: o sinal chega ANTES de a pasta existir.
        ///
        /// Com a armadilha armada antes da criação — que é o que fecha a janela
        /// do caso acima — ela passa a rodar também neste caminho, sobre um
        /// caminho que nunca foi criado. Isso não pode virar erro nem, muito
        /// menos, apagar outra coisa: o `mkdir` travado prende o script
        /// exatamente entre o `trap` e a criação.
        #[test]
        fn queda_antes_de_a_pasta_existir_nao_explode_nem_deixa_rastro() {
            use std::os::unix::process::CommandExt;

            let home = tempfile::tempdir().unwrap();
            let run = tempfile::tempdir().unwrap();
            let Some(bin) = fake_path(home.path(), &["sh", "bash", "base64", "sleep", "env", "rm"])
            else {
                return; // máquina sem um dos utilitários: nada a provar aqui
            };
            let dentro = home.path().join("na-janela");
            if !mkdir_que_trava(&bin, false, &dentro) {
                return; // máquina sem mkdir: nada a provar aqui
            }

            let cmd = integrated(RemoteShell::Bash);
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            let child = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .env_clear()
                .env("PATH", &bin)
                .env("HOME", home.path())
                .env("XDG_RUNTIME_DIR", run.path())
                .env("SHELL", bin.join("bash"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap();

            espera(&dentro, "o script nunca chegou à criação da pasta");
            assert!(
                listagem(run.path()).is_empty(),
                "o mkdir falso não cria nada: a janela sob teste é a de ANTES"
            );

            unsafe { libc::kill(-(child.id() as i32), libc::SIGHUP) };
            let out = child.wait_with_output().unwrap();

            assert!(
                listagem(run.path()).is_empty(),
                "a limpeza de um caminho que não existe não pode criar rastro"
            );
            // O `Hangup` que aparece aqui é o aviso do `sh` que RODA o teste
            // sobre o sinal que ele mesmo recebeu — não é o script falando. O
            // que não pode aparecer é queixa da limpeza.
            let erro = String::from_utf8_lossy(&out.stderr);
            assert!(
                !erro.contains("rm") && !erro.to_lowercase().contains("no such file"),
                "apagar um caminho que nunca existiu é inócuo e silencioso; o \
                 script reclamou: {erro:?}"
            );
        }

        /// Espera o marco aparecer. Prazo generoso de propósito: ele existe para
        /// o teste não pendurar a suíte, não para medir tempo — quem segura o
        /// script do outro lado são os 30 s do utilitário travado.
        fn espera(marco: &std::path::Path, queixa: &str) {
            let ate = std::time::Instant::now();
            while !marco.exists() {
                assert!(
                    ate.elapsed() < std::time::Duration::from_secs(10),
                    "{queixa}"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        fn listagem(dir: &std::path::Path) -> Vec<String> {
            std::fs::read_dir(dir)
                .unwrap()
                .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
                .collect()
        }

        fn roda(shell: RemoteShell) {
            let home = tempfile::tempdir().unwrap();
            let run = tempfile::tempdir().unwrap();
            let program = shell.label();
            let Some(bin) = fake_path(
                home.path(),
                &["sh", program, "base64", "mkdir", "env", "rm"],
            ) else {
                return; // máquina sem um dos utilitários: nada a provar aqui
            };

            let cmd = integrated(shell.clone());
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            let out = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .env_clear()
                .env("PATH", &bin)
                .env("HOME", home.path())
                .env("XDG_RUNTIME_DIR", run.path())
                .env("SHELL", bin.join(program))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    child
                        .stdin
                        .as_mut()
                        .unwrap()
                        .write_all(probe_line(&shell).as_bytes())?;
                    child.wait_with_output()
                })
                .unwrap();

            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains("TYBA_MARK=1:\n"),
                "o shell remoto tem de ter lido o rc do TYBA e saído sem o rastro \
                 da pasta em TYBA_RC_DIR; got: {stdout:?}"
            );
            let leftovers: Vec<String> = std::fs::read_dir(run.path())
                .unwrap()
                .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
                .collect();
            assert!(
                leftovers.is_empty(),
                "regra 9: nada do TYBA pode ficar no servidor depois que a sessão \
                 sobe; sobrou {leftovers:?}"
            );
        }
    }

    /// O eco do terminal remoto, contra um pty de verdade.
    ///
    /// O `ssh -t` aloca um terminal no servidor, e um terminal nasce ecoando o
    /// que se escreve nele. Com o cliente de controle do outro lado, cada
    /// comando que o TYBA manda (`refresh-client`, `send-keys`) voltava pela
    /// rede e chegava ao decodificador como linha não reconhecida — visto no
    /// log do app: `modo de controle do tmux com linha não reconhecida (a
    /// sessão segue): refresh-client -C 168x38`.
    #[cfg(unix)]
    mod sem_eco_no_cliente_de_controle {
        use super::*;
        use parking_lot::Mutex;
        use portable_pty::{native_pty_system, PtySize};
        use std::io::{Read, Write};
        use std::sync::Arc;

        /// Roda `sh -c <script>` num pty de verdade, espera o script chegar ao
        /// ponto em que `marco` aparece, escreve `entrada` e devolve o que o
        /// mestre leu depois disso.
        fn resposta_do_pty(
            script: &str,
            env: &[(&str, std::path::PathBuf)],
            marco: Option<&str>,
            entrada: &str,
        ) -> String {
            let pair = native_pty_system()
                .openpty(PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("openpty");
            let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
            cmd.arg("-c");
            cmd.arg(script);
            cmd.env_clear();
            for (k, v) in env {
                cmd.env(k, v);
            }
            let mut child = pair.slave.spawn_command(cmd).expect("spawn");
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().expect("reader");
            let lido: Arc<Mutex<Vec<u8>>> = Arc::default();
            let acumulando = Arc::clone(&lido);
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    acumulando.lock().extend_from_slice(&buf[..n]);
                }
            });

            if let Some(marco) = marco {
                let ate = std::time::Instant::now();
                while !String::from_utf8_lossy(&lido.lock()).contains(marco) {
                    assert!(
                        ate.elapsed() < std::time::Duration::from_secs(10),
                        "o script nunca chegou ao marco {marco:?}: got {:?}",
                        String::from_utf8_lossy(&lido.lock())
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            lido.lock().clear();

            let mut writer = pair.master.take_writer().expect("writer");
            writer.write_all(entrada.as_bytes()).unwrap();
            writer.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(400));
            let visto = String::from_utf8_lossy(&lido.lock()).into_owned();
            let _ = child.kill();
            let _ = child.wait();
            visto
        }

        const ENTRADA: &str = "refresh-client -C 80x24\n";

        /// A prova de que o harness enxerga o fenômeno: sem ninguém desligar o
        /// eco, o mesmo pty devolve o que foi escrito. Sem isto, o teste de
        /// baixo ficaria verde mesmo medindo nada.
        #[test]
        fn o_pty_ecoa_quando_ninguem_desliga() {
            let visto = resposta_do_pty("cat >/dev/null", &[], None, ENTRADA);
            assert!(
                visto.contains("refresh-client"),
                "um pty sem `stty -echo` devolve o que se escreve nele: {visto:?}"
            );
        }

        #[test]
        fn o_comando_remoto_nao_ecoa_o_que_o_tyba_escreve() {
            let home = tempfile::tempdir().unwrap();
            let run = tempfile::tempdir().unwrap();
            let Some(bin) = no_servidor_de_mentira::fake_path(
                home.path(),
                &["sh", "bash", "base64", "mkdir", "env", "rm", "stty", "cat"],
            ) else {
                return; // máquina sem um dos utilitários: nada a provar aqui
            };
            // Um tmux que responde "a sessão existe" e depois só consome a
            // entrada: o que está sob teste é o terminal, não o tmux.
            let tmux = bin.join("tmux");
            std::fs::write(
                &tmux,
                "#!/bin/sh\ncase \" $* \" in *has-session*) exit 0 ;; esac\nexec cat >/dev/null\n",
            )
            .unwrap();
            std::fs::set_permissions(&tmux, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();

            let cmd = integrated(RemoteShell::Bash);
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            let visto = resposta_do_pty(
                script,
                &[
                    ("PATH", bin.clone()),
                    ("HOME", home.path().to_path_buf()),
                    ("XDG_RUNTIME_DIR", run.path().to_path_buf()),
                    ("SHELL", bin.join("bash")),
                ],
                Some("tyba-ctl="),
                ENTRADA,
            );
            assert!(
                !visto.contains("refresh-client"),
                "o comando do TYBA não pode voltar ecoado: ele atravessa a rede \
                 duas vezes e chega ao decodificador como linha não reconhecida; \
                 got {visto:?}"
            );
        }
    }

    #[test]
    fn o_script_remoto_cabe_no_envelope_de_sh_c() {
        for shell in [RemoteShell::Bash, RemoteShell::Zsh] {
            let cmd = integrated(shell.clone());
            assert!(
                cmd.starts_with("sh -c '") && cmd.ends_with('\''),
                "{}: o login shell remoto só pode ver sh -c",
                shell.label()
            );
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            assert!(
                !script.contains('\''),
                "{}: aspas simples fecham o envelope no meio",
                shell.label()
            );
            assert!(
                !cmd.contains('\n') && !cmd.contains('!') && !cmd.contains("\\\\"),
                "{}: newline quebra csh, `!` liga history expansion",
                shell.label()
            );
        }
    }
}

//! Onde o app pode — e onde não pode — tocar no disco.
//!
//! O módulo responde a **duas** perguntas, e elas não são a mesma. Compartilham
//! a lista de prefixos e a regra de comparação, mas cada uma é calibrada pelo
//! preço que o **seu** chamador paga quando a resposta erra:
//!
//! - [`reopen_policy`] — chamador `resume_startup`, uma sessão de shell por vez.
//!   Adiar à toa custa uma aba que não volta; tocar num mount morto pendura a
//!   thread de boot.
//! - [`may_hang_shared_thread`] — chamador `session_worktree_roots`, na thread
//!   `repo-reconcile`. Adiar à toa custa o chip de branch de um repositório até
//!   o tick seguinte; tocar num mount morto pendura a thread de **todos**.
//!
//! A assimetria é o ponto: a segunda desiste de mais caminhos que a primeira, e
//! é por isso que existem as duas. Reusar uma no lugar da outra já foi um bug —
//! `session_worktree_roots` perguntava pela `reopen_policy` e mandava o `git`
//! para dentro de `/mnt`, onde um NFS morto congelava o chip do app inteiro.
//!
//! **Não reintroduza um `stat` incondicional no reopen.** Parece descuido; não é.
//!
//! O `Info.plist` do TYBA declara cinco permissões de pasta (Desktop, Documents,
//! Downloads, volume removível, volume de rede). No macOS, tocar numa pasta
//! dessas abre o diálogo do TCC — e o TCC **segura a thread que chamou até o
//! usuário clicar**. Era isso que travava a abertura: `resume_startup` fazia
//! `cwd.is_dir()` em cada sessão morta, dentro do `.setup()`, que roda na main
//! thread com o event loop parado. O resultado era o splash na tela, a janela
//! sem responder e o diálogo por cima. Não era lentidão — era um `stat`.
//!
//! Só aparece na primeira execução depois de um build novo, porque depois disso
//! a permissão já foi concedida. Num app distribuído, isso é *toda atualização*.
//!
//! **O que consertou o congelamento foi tirar o boot da main thread** (ver
//! [`crate::boot`]) — não esta classificação. Com o boot em thread própria, o
//! diálogo segura só aquela thread e a janela continua respondendo. Nenhuma
//! variante daqui faz o diálogo do TCC deixar de aparecer, e a de
//! [`ReopenPolicy::Unchecked`] explica por que não dá.
//!
//! O que esta classificação resolve é o outro risco, o que thread nenhuma
//! salva: num volume de rede que não responde, o `stat` pendura em I/O
//! ininterrompível — sem diálogo para clicar e sem timeout para estourar. Contra
//! isso a única defesa é não tocar, e é o que [`ReopenPolicy::Skip`] faz.

use std::path::{Path, PathBuf};

/// O que o arranque faz com o cwd de uma sessão de shell morta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenPolicy {
    /// Caminho comum: confere que a pasta existe e reabre. É o que sempre foi.
    Checked,
    /// Pasta sob TCC: `resume_startup` reabre sem conferir antes.
    ///
    /// **Isto não evita o diálogo do TCC — nem evitou algum dia.** Reabrir a
    /// sessão *é* entrar na pasta: logo adiante, `session::resolve_cwd` stata o
    /// mesmo caminho e o shell nasce com `chdir` para dentro dele. O acesso
    /// acontece de todo jeito; pular o `is_dir()` daqui só muda qual chamada
    /// esbarra primeiro no TCC. E o prompt é um por categoria de pasta, não um
    /// por sessão — não existe um N de diálogos para reduzir.
    ///
    /// O que a variante muda de fato é o destino quando a pasta sumiu:
    /// `Checked` descarta a tab, `Unchecked` reabre em `$HOME`, que é onde o
    /// `resolve_cwd` cai. Numa pasta que o usuário só visita de vez em quando,
    /// devolver a tab viva vale mais que a distinção.
    ///
    /// E não adianta perseguir o "zero syscall" propagando a política até o
    /// `resolve_cwd`: sem aquele `is_dir()` o `chdir` do spawn falha numa pasta
    /// que sumiu, e a tab não volta nem no home — pior que hoje, e sem ganho,
    /// porque o `chdir` entra na pasta protegida do mesmo jeito.
    Unchecked,
    /// Volume que pode não estar montado: não reabre, e não toca.
    ///
    /// A única variante que de fato evita a syscall, porque é a única que
    /// desiste da sessão — quem reabre acaba tocando na pasta em algum ponto.
    /// Aqui nem o spawn serve de plano B: num mount morto quem pendura é a
    /// própria chamada, stat ou chdir, sem diálogo e sem timeout. A sessão volta
    /// morta — o que não é perda, porque um shell num volume ausente não teria
    /// como funcionar de qualquer jeito.
    Skip,
}

/// As três pastas do `Info.plist` que ficam dentro do home.
const TCC_HOME_DIRS: &[&str] = &["Desktop", "Documents", "Downloads"];

/// Onde volume removível e volume de rede aparecem montados, e **só** o que é
/// ponto de montagem por definição. As outras duas permissões do `Info.plist` —
/// e o caso sem diálogo, que é o que pendura.
///
/// Um caminho aqui dentro é mount de rede ou de mídia, ponto: não existe versão
/// "local e viva" de `/Volumes`. Por isso as duas perguntas do módulo desistem
/// dele. `/mnt` e `/media` não têm essa garantia e moram em
/// [`AMBIGUOUS_MOUNT_PREFIXES`].
const MOUNT_PREFIXES: &[&str] = &["/Volumes", "/Network", "/net"];

/// Onde tanto pode estar um NFS/SMB morto quanto um disco local que funciona.
///
/// No WSL o disco do Windows mora em `/mnt/c`, e é lá que muita gente guarda
/// repositório; `/media` é o automount de pendrive do Linux. Tratar isto como
/// volume ausente cobra um preço de quem está com tudo montado e funcionando —
/// então quem decide não é a lista, é o custo de errar do chamador:
/// [`reopen_policy`] deixa passar (erraria uma aba), [`may_hang_shared_thread`]
/// desiste (erraria a thread de todo mundo).
const AMBIGUOUS_MOUNT_PREFIXES: &[&str] = &["/mnt", "/media"];

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// `Path::starts_with` compara componente a componente, e é isso que faz
/// `~/Documents-old` NÃO casar com `~/Documents`. Um `starts_with` de string
/// adiaria a pasta errada.
fn under_any(cwd: &Path, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| cwd.starts_with(prefix))
}

/// Compartilhamento de rede do Windows: `\\servidor\share` e a forma estendida
/// `\\?\UNC\servidor\share`. É mount de rede tanto quanto `/Volumes` — some
/// junto com a VPN e pendura igual —, e nenhum prefixo POSIX o alcança.
///
/// A comparação é no texto, e não com `Path::starts_with`, porque o separador de
/// componente é o **da plataforma que compilou**: num binário de macOS ou Linux,
/// `\\servidor\share` é um componente só, e todo prefixo erra. Caminho de
/// Windows precisa dos dois separadores em qualquer plataforma — o `git` de lá
/// devolve `//servidor/share`, e a barra invertida é o que o usuário digita.
///
/// `\\?\C:\...` (prefixo de caminho longo, disco local) e `\\.\pipe\...`
/// (namespace de device) também começam com dois separadores e **não** são
/// share: ficam de fora.
///
/// Em POSIX, duas barras iniciais são um alias legal de uma só, então `//tmp/x`
/// cai aqui por engano — e este é o único ponto do módulo em que os dois
/// chamadores pagam o mesmo falso positivo: uma aba que não reabre, o chip de um
/// repositório até o tick seguinte. Contra isso, deixar um `\\servidor` passar
/// pendura a thread de reconciliação para sempre. Ninguém tem worktree em
/// `//tmp`; quem trabalha em share de rede existe.
fn is_unc(cwd: &Path) -> bool {
    let raw = cwd.as_os_str().to_string_lossy();
    let separator = |c: char| c == '\\' || c == '/';
    let Some(rest) = raw
        .strip_prefix(separator)
        .and_then(|tail| tail.strip_prefix(separator))
    else {
        return false;
    };
    if let Some(tail) = rest
        .strip_prefix('?')
        .and_then(|tail| tail.strip_prefix(separator))
    {
        // `\\?\UNC\servidor\share` é share; `\\?\C:\...` é o disco local.
        return tail
            .split(separator)
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("UNC"));
    }
    // `\\.\` é device, não share.
    !rest.starts_with('.')
}

/// Função pura: decide olhando só para o texto do caminho, sem syscall. Statar
/// para descobrir se pode statar seria o próprio bug — é num volume morto que a
/// classificação precisa acertar, e é lá que a syscall pendura.
///
/// **Chamador: `resume_startup`, na thread de boot, uma sessão de shell por
/// vez.** Errar para o lado de adiar custa uma aba que não volta, e o dono
/// reabre na mão; errar para o lado de tocar pendura a thread de boot. Com esse
/// par de custos, `/mnt` e `/media` seguem o caminho comum: quase sempre são
/// disco local vivo, e adiá-los cobraria a aba de todo usuário de WSL.
///
/// **Quem paga mais caro por pendurar não usa esta função.** Use
/// [`may_hang_shared_thread`].
pub fn reopen_policy(cwd: &Path, home: Option<&Path>) -> ReopenPolicy {
    if is_unc(cwd) || under_any(cwd, MOUNT_PREFIXES) {
        return ReopenPolicy::Skip;
    }

    if let Some(home) = home {
        for dir in TCC_HOME_DIRS {
            if cwd.starts_with(home.join(dir)) {
                return ReopenPolicy::Unchecked;
            }
        }
    }

    ReopenPolicy::Checked
}

/// A outra pergunta do módulo: **este caminho pode pendurar uma syscall numa
/// thread que não é só dele?** Pura pelo mesmo motivo da [`reopen_policy`].
///
/// **Chamador: `session_worktree_roots`, na thread `repo-reconcile`.** Ali o
/// `repo::toplevel` faz shell-out de `git rev-parse` e bloqueia no `output()`.
/// Num NFS/SMB morto o `git` fica em I/O ininterrompível — sem diálogo para
/// clicar e sem timeout para estourar —, a thread nunca volta e o
/// `EVENT_RECONCILED` para de sair para **todos** os repositórios, não só para o
/// do caminho ruim.
///
/// Os custos são o inverso dos da [`reopen_policy`], e é isso que separa as
/// duas: adiar à toa custa aqui o chip de branch de **um** repositório até o
/// tick seguinte — a reconciliação reexecuta, não é decisão definitiva —,
/// enquanto tocar errado custa o chip de todos e não tem tick seguinte. Com essa
/// conta, `/mnt` e `/media` entram. A lista não mudou de opinião sobre o WSL: o
/// chamador é que paga outra conta.
///
/// Pasta protegida pelo TCC fica de fora, e por isso esta função não precisa do
/// home: lá existe um diálogo para clicar e o bloqueio acaba. É a diferença
/// entre esperar e pendurar.
pub fn may_hang_shared_thread(cwd: &Path) -> bool {
    is_unc(cwd) || under_any(cwd, MOUNT_PREFIXES) || under_any(cwd, AMBIGUOUS_MOUNT_PREFIXES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chamador `resume_startup`: uma sessão de shell por vez, e errar custa uma
    /// aba.
    fn policy(cwd: &str) -> ReopenPolicy {
        reopen_policy(Path::new(cwd), Some(Path::new("/Users/tester")))
    }

    /// Chamador `session_worktree_roots`, na thread `repo-reconcile`: errar
    /// custa o chip de todos os repositórios.
    fn hangs(cwd: &str) -> bool {
        may_hang_shared_thread(Path::new(cwd))
    }

    #[test]
    fn caminho_comum_continua_sendo_conferido() {
        assert_eq!(policy("/Users/tester/code/tyba"), ReopenPolicy::Checked);
        assert_eq!(policy("/tmp/scratch"), ReopenPolicy::Checked);
        assert_eq!(policy("/Users/tester"), ReopenPolicy::Checked);
    }

    /// "Sem conferir" é sobre o `is_dir()` do `resume_startup`, e só. O `stat` do
    /// `resolve_cwd` vem depois — ver a doc de [`ReopenPolicy::Unchecked`].
    #[test]
    fn as_tres_pastas_do_info_plist_reabrem_sem_conferir() {
        for dir in TCC_HOME_DIRS {
            assert_eq!(
                policy(&format!("/Users/tester/{dir}")),
                ReopenPolicy::Unchecked,
                "{dir} está no Info.plist e o arranque não a confere antes de reabrir"
            );
            assert_eq!(
                policy(&format!("/Users/tester/{dir}/projeto/sub")),
                ReopenPolicy::Unchecked,
            );
        }
    }

    /// A armadilha que um `starts_with` de string cometeria.
    #[test]
    fn pasta_irma_de_nome_parecido_nao_e_confundida_com_a_protegida() {
        assert_eq!(
            policy("/Users/tester/Documents-old/projeto"),
            ReopenPolicy::Checked
        );
        assert_eq!(policy("/Users/tester/Downloadsx"), ReopenPolicy::Checked);
        assert_eq!(policy("/Users/tester/Desktop.bak"), ReopenPolicy::Checked);
    }

    #[test]
    fn volume_montado_nao_e_reaberto_no_arranque() {
        assert_eq!(policy("/Volumes/NAS/repo"), ReopenPolicy::Skip);
        assert_eq!(policy("/Volumes"), ReopenPolicy::Skip);
        assert_eq!(policy("/Network/Servers/build"), ReopenPolicy::Skip);
        assert_eq!(policy("/net/host/share"), ReopenPolicy::Skip);
    }

    /// Chamador `resume_startup`. `/mnt/c` é o disco do Windows no WSL: adiar
    /// aquilo devolveria tab morta para quem está com tudo montado, e uma aba
    /// perdida é o preço que este chamador se recusa a pagar.
    #[test]
    fn reabrir_sessao_deixa_mnt_e_media_no_caminho_comum() {
        assert_eq!(policy("/mnt/c/Users/tester/code"), ReopenPolicy::Checked);
        assert_eq!(policy("/media/usb/repo"), ReopenPolicy::Checked);
    }

    /// Chamador `session_worktree_roots`. Mesmo caminho, resposta oposta: aqui o
    /// `git rev-parse` bloqueia a thread `repo-reconcile`, e um `/mnt/nas` de
    /// NFS morto leva junto o chip de branch de todos os outros repositórios.
    #[test]
    fn thread_compartilhada_adia_mnt_e_media() {
        assert!(hangs("/mnt/nas/repo"));
        assert!(hangs("/mnt/c/Users/tester/code"));
        assert!(hangs("/media/usb/repo"));
    }

    /// O achado que produziu as duas funções, dito num assert só: o mesmo texto,
    /// duas respostas, porque os chamadores pagam contas diferentes. Reusar a
    /// política de um no outro é o bug.
    #[test]
    fn as_duas_perguntas_divergem_no_mnt_do_wsl() {
        let wsl = "/mnt/c/Users/tester/code";
        assert_eq!(policy(wsl), ReopenPolicy::Checked, "a aba do WSL volta");
        assert!(hangs(wsl), "a reconciliação não manda o git para lá");
    }

    /// Quem paga caro desiste de tudo que o barato já desiste. O contrário não
    /// vale — é a assimetria inteira do módulo.
    #[test]
    fn thread_compartilhada_adia_tudo_que_o_arranque_ja_adiava() {
        for cwd in [
            "/Volumes/NAS/repo",
            "/Network/Servers/build",
            "/net/host/share",
            r"\\servidor\share\repo",
        ] {
            assert_eq!(policy(cwd), ReopenPolicy::Skip, "{cwd}");
            assert!(hangs(cwd), "{cwd}");
        }
    }

    /// A discriminação: o filtro caro não é "descarta tudo". Caminho local
    /// segue tocável, e pasta do TCC também — lá o diálogo tem um botão, e
    /// esperar por um clique não é pendurar.
    #[test]
    fn thread_compartilhada_toca_caminho_local_e_pasta_do_tcc() {
        assert!(!hangs("/Users/tester/code/tyba"));
        assert!(!hangs("/tmp/scratch"));
        assert!(!hangs("/Users/tester/Documents/tyba"));
        assert!(
            !hangs("/mntx/repo"),
            "componente inteiro, não prefixo de texto"
        );
        assert!(!hangs("C:/Users/tester/code"));
    }

    /// Share do Windows é volume de rede escrito com outro separador: some com a
    /// VPN e pendura igual a `/Volumes`. Vale para os dois chamadores, e nenhum
    /// prefixo POSIX o alcançava.
    #[test]
    fn caminho_unc_do_windows_conta_como_volume_de_rede() {
        for cwd in [
            r"\\servidor\share\repo",
            "//servidor/share/repo",
            r"\\?\UNC\servidor\share\repo",
            r"\\servidor",
        ] {
            assert_eq!(policy(cwd), ReopenPolicy::Skip, "{cwd}");
            assert!(hangs(cwd), "{cwd}");
        }
    }

    /// Duas barras iniciais nem sempre são share: o prefixo de caminho longo do
    /// Windows aponta para o disco local, e `\\.\` é namespace de device.
    /// Classificá-los como rede tiraria o chip de um repositório local.
    #[test]
    fn prefixo_de_caminho_longo_e_device_nao_sao_share() {
        for cwd in [r"\\?\C:\Users\tester\code", r"\\.\pipe\tyba"] {
            assert_eq!(policy(cwd), ReopenPolicy::Checked, "{cwd}");
            assert!(!hangs(cwd), "{cwd}");
        }
    }

    /// Volume vence pasta protegida: um home montado em `/Volumes` pode não
    /// responder, e pendurar é pior que pedir permissão.
    #[test]
    fn volume_tem_precedencia_sobre_pasta_do_home() {
        assert_eq!(
            reopen_policy(
                Path::new("/Volumes/Externo/Documents/projeto"),
                Some(Path::new("/Volumes/Externo"))
            ),
            ReopenPolicy::Skip
        );
    }

    /// O caso do relatório: um conjunto de sessões restauradas, e quem é
    /// conferido, quem é reaberto às cegas e quem fica para depois.
    #[test]
    fn um_boot_com_sessoes_variadas_se_reparte_nos_tres_destinos() {
        let restored = [
            "/Users/tester/code/tyba",
            "/Users/tester/Documents/notas",
            "/Volumes/NAS/build",
            "/tmp/scratch",
            "/Users/tester/Desktop",
            "/Network/Servers/ci",
            "/Users/tester/Documents-old/arquivo",
        ];

        let mut checked = Vec::new();
        let mut unchecked = Vec::new();
        let mut skipped = Vec::new();
        for cwd in restored {
            match policy(cwd) {
                ReopenPolicy::Checked => checked.push(cwd),
                ReopenPolicy::Unchecked => unchecked.push(cwd),
                ReopenPolicy::Skip => skipped.push(cwd),
            }
        }

        assert_eq!(
            checked,
            [
                "/Users/tester/code/tyba",
                "/tmp/scratch",
                "/Users/tester/Documents-old/arquivo",
            ]
        );
        assert_eq!(
            unchecked,
            ["/Users/tester/Documents/notas", "/Users/tester/Desktop"]
        );
        assert_eq!(skipped, ["/Volumes/NAS/build", "/Network/Servers/ci"]);
    }

    #[test]
    fn sem_home_nao_da_para_reconhecer_pasta_protegida() {
        assert_eq!(
            reopen_policy(Path::new("/Users/tester/Documents"), None),
            ReopenPolicy::Checked
        );
        // O volume não depende do home, então continua reconhecido.
        assert_eq!(
            reopen_policy(Path::new("/Volumes/NAS"), None),
            ReopenPolicy::Skip
        );
    }
}

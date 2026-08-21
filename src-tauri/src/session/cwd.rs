//! Onde o arranque pode — e onde não pode — tocar no disco.
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

/// Onde volume removível e volume de rede aparecem montados. As outras duas
/// permissões do `Info.plist` — e o caso sem diálogo, que é o que pendura.
///
/// Sem `/mnt` e sem `/media` de propósito: no WSL o disco do Windows mora em
/// `/mnt/c`, e é lá que muita gente guarda repositório. Adiar aquilo seria
/// devolver a tab morta para quem está com tudo montado e funcionando.
const MOUNT_PREFIXES: &[&str] = &["/Volumes", "/Network", "/net"];

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// Função pura: decide olhando só para o texto do caminho, sem syscall. Statar
/// para descobrir se pode statar seria o próprio bug — é num volume morto que a
/// classificação precisa acertar, e é lá que a syscall pendura.
pub fn reopen_policy(cwd: &Path, home: Option<&Path>) -> ReopenPolicy {
    // `Path::starts_with` compara componente a componente, e é isso que faz
    // `~/Documents-old` NÃO casar com `~/Documents`. Um `starts_with` de string
    // adiaria a pasta errada.
    for prefix in MOUNT_PREFIXES {
        if cwd.starts_with(prefix) {
            return ReopenPolicy::Skip;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(cwd: &str) -> ReopenPolicy {
        reopen_policy(Path::new(cwd), Some(Path::new("/Users/tester")))
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

    /// `/mnt/c` é o disco do Windows no WSL: adiar aquilo devolveria tab morta
    /// para quem está com tudo montado.
    #[test]
    fn mnt_e_media_seguem_o_caminho_comum() {
        assert_eq!(policy("/mnt/c/Users/tester/code"), ReopenPolicy::Checked);
        assert_eq!(policy("/media/usb/repo"), ReopenPolicy::Checked);
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

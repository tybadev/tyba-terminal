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
//! Tirar o boot da main thread (ver [`crate::boot`]) impede o congelamento, mas
//! não resolve o resto: pedir acesso a Documents porque o app abriu, e não
//! porque alguém pediu aquela sessão, continua errado. E o volume de rede é pior
//! que o diálogo — num mount que não responde, o `stat` pendura em I/O
//! ininterrompível, sem diálogo nenhum para clicar e sem caminho de saída.
//!
//! Daí as três respostas desta classificação, uma para cada risco.

use std::path::{Path, PathBuf};

/// O que o arranque faz com o cwd de uma sessão de shell morta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenPolicy {
    /// Caminho comum: confere que a pasta existe e reabre. É o que sempre foi.
    Checked,
    /// Pasta sob TCC: reabre **sem** conferir antes.
    ///
    /// A conferência só produzia um booleano, e custava um diálogo do sistema.
    /// O `resolve_cwd` do spawn já cai em `$HOME` quando a pasta sumiu, então o
    /// que se perde ao não conferir é a distinção entre "não abre a tab" e "abre
    /// a tab no home" — e isso não vale um prompt de permissão no arranque.
    Unchecked,
    /// Volume que pode não estar montado: não reabre, e não toca.
    ///
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

/// Função pura: decide olhando só para o texto do caminho, sem syscall — que é
/// o ponto inteiro deste módulo.
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

    #[test]
    fn as_tres_pastas_do_info_plist_reabrem_sem_stat() {
        for dir in TCC_HOME_DIRS {
            assert_eq!(
                policy(&format!("/Users/tester/{dir}")),
                ReopenPolicy::Unchecked,
                "{dir} está no Info.plist e não pode ser conferida no arranque"
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

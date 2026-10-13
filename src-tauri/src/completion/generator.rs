//! Os provedores que perguntam ao `git`: branch, remoto, arquivo mudado.
//!
//! Não são os `generators` do Fig — aqueles são script de terceiro rodando no
//! shell a cada tecla, e o ADR os recusou. Aqui a lista é FECHADA e é nossa:
//! três consultas de leitura, cada uma com argumentos fixos, montadas como
//! vetor de argumentos (nunca uma string de shell), dentro da jaula read-only
//! do git (`worktree::git_in`) e com prazo. Passou do prazo, o processo morre
//! e a lista sai vazia: completação atrasada é pior que completação faltando.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// O prazo de cada consulta. `for-each-ref` num repo grande leva poucos ms;
/// meio segundo é folga para disco frio, e ainda cabe numa tecla.
pub const TIMEOUT: Duration = Duration::from_millis(500);

/// O que se pergunta ao git.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitQuery {
    /// Branches locais e remotas.
    Branches,
    /// Os remotos configurados (`origin`, `upstream`).
    Remotes,
    /// Arquivos modificados ou novos, relativos ao diretório da consulta.
    ChangedFiles,
}

/// Os argumentos de cada consulta — fixos, e só de leitura.
///
/// Função pura de propósito: é ela que o teste confere, porque é aqui que um
/// descuido viraria escrita (`branch -D`) ou rede (`fetch`). Nenhum valor vem
/// do que o usuário digitou.
///
/// Saída com NUL sempre que o git oferece (regra 8 do CLAUDE.md). `git remote`
/// não tem `-z`; nome de remoto não pode ter quebra de linha, então a linha
/// basta.
pub fn git_args(query: GitQuery) -> &'static [&'static str] {
    match query {
        GitQuery::Branches => &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00",
            "refs/heads",
            "refs/remotes",
        ],
        GitQuery::Remotes => &["remote"],
        // Relativo ao cwd por padrão: `git add ` dentro de `src/` oferece
        // `main.rs`, não `src/main.rs` — que nem resolveria dali.
        GitQuery::ChangedFiles => &[
            "ls-files",
            "--modified",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    }
}

/// Os nomes, na ordem em que o git os deu, sem vazio nem repetição.
///
/// `ls-files --modified --others` lista duas vezes o arquivo apagado do disco
/// que ainda está no índice; a lista mostra uma.
pub fn parse(query: GitQuery, raw: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(raw);
    if query == GitQuery::Branches {
        return parse_branches(&text);
    }
    let (sep, trim) = match query {
        GitQuery::Remotes => ('\n', true),
        // Nome de arquivo pode começar ou terminar com espaço; não se apara.
        GitQuery::Branches | GitQuery::ChangedFiles => ('\0', false),
    };
    let mut found: Vec<String> = Vec::new();
    for item in text.split(sep) {
        let item = if trim { item.trim() } else { item };
        if !item.is_empty() && !found.iter().any(|seen| seen == item) {
            found.push(item.to_string());
        }
    }
    found
}

/// Branches locais e remotas, com a remota escondida quando existe a local de
/// mesmo nome — `origin/main` ao lado de `main` é ruído num `checkout`. A
/// mesma regra do painel de branches (`worktree::branches::parse_branches`).
fn parse_branches(text: &str) -> Vec<String> {
    let mut locals: Vec<String> = Vec::new();
    let mut remotes: Vec<(String, String)> = Vec::new();
    for item in text.split('\0') {
        // `for-each-ref` termina cada registro com o NUL do formato E com a
        // própria quebra de linha; o `trim` tira a quebra que sobra. Nome de
        // ref não tem espaço nas pontas.
        let item = item.trim();
        if let Some(name) = item.strip_prefix("refs/heads/") {
            locals.push(name.to_string());
        } else if let Some(rest) = item.strip_prefix("refs/remotes/") {
            let Some((_, short)) = rest.split_once('/') else {
                continue;
            };
            // `origin/HEAD` é um symref, não uma branch em que se possa entrar.
            if short != "HEAD" {
                remotes.push((rest.to_string(), short.to_string()));
            }
        }
    }
    let mut found = locals.clone();
    for (name, short) in remotes {
        if !locals.contains(&short) && !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// Roda a consulta em `dir`, dentro da jaula read-only do git, com prazo.
pub fn run(query: GitQuery, dir: &Path) -> Vec<String> {
    let mut cmd = crate::worktree::git_in(dir);
    cmd.args(git_args(query));
    match run_with_timeout(cmd, TIMEOUT) {
        Some(raw) => parse(query, &raw),
        None => Vec::new(),
    }
}

/// O stdout do processo, ou `None` se ele falhou ou passou do prazo.
///
/// O stdout é lido numa thread: um repo com milhares de arquivos novos enche o
/// pipe, e esperar o processo sem ler o pipe travaria os dois.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<Vec<u8>> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().unwrap_or_default();
                return status.success().then_some(out);
            }
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nada que escreva, nada que vá à rede. A lista de verbos é fechada: um
    /// provedor novo que quisesse `fetch` teria de mudar este teste.
    #[test]
    fn as_consultas_sao_so_de_leitura() {
        for query in [
            GitQuery::Branches,
            GitQuery::Remotes,
            GitQuery::ChangedFiles,
        ] {
            let verbo = git_args(query)[0];
            assert!(
                matches!(verbo, "for-each-ref" | "remote" | "ls-files"),
                "{query:?} usa `{verbo}`"
            );
            // `git remote` sem subcomando só lista; com `add`/`remove` escreveria.
            if verbo == "remote" {
                assert_eq!(git_args(query).len(), 1);
            }
        }
    }

    #[test]
    fn branch_e_arquivo_saem_separados_por_nul() {
        assert!(git_args(GitQuery::Branches)
            .iter()
            .any(|a| a.contains("%00")));
        assert!(git_args(GitQuery::ChangedFiles).contains(&"-z"));
    }

    #[test]
    fn parse_de_branch_tira_o_head_simbolico_e_a_remota_repetida() {
        let raw = b"refs/heads/main\0\nrefs/heads/feat/x\0\nrefs/remotes/origin/HEAD\0\n\
refs/remotes/origin/main\0\nrefs/remotes/origin/fix/y\0\n";
        assert_eq!(
            parse(GitQuery::Branches, raw),
            vec!["main", "feat/x", "origin/fix/y"],
            "origin/main some porque main existe; origin/HEAD é symref"
        );
    }

    #[test]
    fn parse_de_arquivo_aceita_espaco_e_acento_e_nao_repete() {
        // Nome com espaço e acento é por isso que a saída é NUL e não linha.
        let raw = "notas de reunião.md\0src/main.rs\0notas de reunião.md\0".as_bytes();
        assert_eq!(
            parse(GitQuery::ChangedFiles, raw),
            vec!["notas de reunião.md", "src/main.rs"]
        );
    }

    #[test]
    fn parse_de_remoto_e_por_linha() {
        assert_eq!(
            parse(GitQuery::Remotes, b"origin\nupstream\n"),
            vec!["origin", "upstream"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn processo_que_passa_do_prazo_morre_e_devolve_nada() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let start = Instant::now();
        assert!(run_with_timeout(cmd, Duration::from_millis(50)).is_none());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn processo_que_falha_devolve_nada() {
        assert!(run_with_timeout(Command::new("false"), TIMEOUT).is_none());
    }

    /// Ponta a ponta, num repo de verdade criado no teste.
    #[test]
    fn consulta_num_repo_de_verdade() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        git(&["add", "a.txt"]);
        git(&["commit", "-qm", "init"]);
        git(&["branch", "feat/y"]);
        git(&["remote", "add", "origin", "https://example.invalid/r.git"]);
        std::fs::write(repo.join("a.txt"), "2").unwrap();
        std::fs::write(repo.join("novo.txt"), "").unwrap();

        let mut branches = run(GitQuery::Branches, repo);
        branches.sort();
        assert_eq!(branches, vec!["feat/y", "main"]);
        assert_eq!(run(GitQuery::Remotes, repo), vec!["origin"]);
        let mut files = run(GitQuery::ChangedFiles, repo);
        files.sort();
        assert_eq!(files, vec!["a.txt", "novo.txt"]);
    }
}

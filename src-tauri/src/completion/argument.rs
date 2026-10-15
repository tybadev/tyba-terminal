//! De onde vêm os candidatos de argumento.
//!
//! Até aqui a completação de argumento tinha uma fonte só: o que o dono já
//! digitou (`super::next_tokens`). Isso completa `git c` porque ele já usou
//! `git commit`, e não completa nada numa máquina onde ele nunca rodou
//! `docker` — a lista nasce vazia justamente quando ele mais precisaria dela.
//!
//! Aqui o dado vem de quem sabe: o git diz quais branches existem, o
//! `package.json` diz quais scripts existem, o `Makefile` diz quais alvos
//! existem. O histórico deixa de ser a **fonte** e vira a **ordem** — o que já
//! foi usado sobe, o resto continua na lista em vez de sumir.
//!
//! Nada disso executa regra de terceiro. As specs do Fig têm `generators`, que
//! rodam comando no shell a cada tecla para produzir sugestão; importar aquilo
//! seria deixar um banco de dados de terceiro executar comandos no terminal do
//! dono, que é a classe de coisa que o produto recusa. Os provedores aqui são
//! código nosso, no core, e cada um sabe exatamente o que lê.

use std::path::Path;

/// Quem responde pelo token que está sendo digitado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Branches locais e remotas do repositório.
    GitBranch,
    /// Remotos configurados (`origin`, `upstream`).
    GitRemote,
    /// Arquivos modificados ou novos — o que `git add` quer.
    GitChangedFile,
    /// Chaves de `scripts` no `package.json`.
    NpmScript,
    /// Alvos declarados no `Makefile`.
    MakeTarget,
    /// Hosts configurados no gestor de conexões.
    SshHost,
    /// Nomes de containers em execução.
    DockerContainer,
}

/// As palavras do prefixo que decidem o provedor, já sem flags.
///
/// Flag é descartada porque `git -c core.x=1 checkout ` continua sendo um
/// checkout. **Limitação conhecida**: valor de flag separado por espaço (o `/x`
/// de `git -C /x`) vira palavra e desloca a contagem — o provedor deixa de
/// casar e a completação cai no histórico, que é degradar, não errar.
fn words(prefix: &str) -> Vec<&str> {
    prefix
        .split_whitespace()
        .filter(|w| !w.starts_with('-'))
        .collect()
}

/// Só o nome do binário, sem o caminho: `/usr/bin/git` e `git` são o mesmo.
fn binary(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// A tabela. Declarativa de propósito: acrescentar um comando é acrescentar uma
/// linha, e é isso que mantém a lista auditável.
///
/// O prefixo precisa terminar em espaço — `git checkout` sem espaço ainda está
/// completando o **subcomando**, não o argumento dele.
pub fn provider_for(prefix: &str) -> Option<Provider> {
    if !prefix.ends_with(char::is_whitespace) {
        return None;
    }
    let words = words(prefix);
    let head = binary(words.first()?);
    let sub = words.get(1).copied().unwrap_or("");

    match (head, sub) {
        ("git", "checkout" | "switch" | "merge" | "rebase" | "cherry-pick") => {
            Some(Provider::GitBranch)
        }
        // `git branch` só oferece branch depois de uma flag que age sobre uma
        // existente. `git branch <nome>` está criando, e sugerir um nome que já
        // existe ali é sugerir um erro.
        ("git", "branch") if prefix.contains(" -d") || prefix.contains(" -D") => {
            Some(Provider::GitBranch)
        }
        // `git push origin feat/x`: primeiro o remoto, depois a branch.
        ("git", "push" | "pull" | "fetch") => match words.len() {
            2 => Some(Provider::GitRemote),
            3 => Some(Provider::GitBranch),
            _ => None,
        },
        // Só os subcomandos que agem sobre um remoto que JÁ existe. `remote add`
        // cria: oferecer `origin` ali é sugerir colisão.
        ("git", "remote")
            if words.len() == 3
                && matches!(
                    words[2],
                    "remove" | "rm" | "rename" | "set-url" | "get-url" | "show" | "prune"
                ) =>
        {
            Some(Provider::GitRemote)
        }
        ("git", "add" | "restore") => Some(Provider::GitChangedFile),
        ("npm" | "pnpm" | "yarn" | "bun", "run") => Some(Provider::NpmScript),
        // Só subcomando que age sobre container que JÁ existe. `docker run` cria
        // um do zero: oferecer o nome de um existente ali é sugerir colisão.
        (
            "docker",
            "exec" | "logs" | "stop" | "start" | "restart" | "attach" | "kill" | "inspect" | "top",
        ) => Some(Provider::DockerContainer),
        ("make", _) => Some(Provider::MakeTarget),
        ("ssh" | "scp" | "sftp", _) if words.len() == 1 => Some(Provider::SshHost),
        _ => None,
    }
}

/// Alvos declarados num `Makefile`.
///
/// Regra deliberadamente estreita: alvo é o que aparece no começo da linha,
/// antes de `:` que não é `:=`. Fica de fora regra com variável no nome
/// (`$(BIN):`), porque expandi-la exigiria interpretar o Makefile — e um alvo
/// inventado é pior que um alvo faltando.
pub fn parse_make_targets(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines() {
        // Continuação, receita e comentário nunca declaram alvo.
        if line.starts_with(['\t', '#', ' ']) {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        // `:=` e `::=` são atribuição de variável, não alvo.
        if line[colon..].starts_with(":=") || line[colon..].starts_with("::=") {
            continue;
        }
        let name = line[..colon].trim();
        if name.is_empty() || name.starts_with('.') || name.contains('$') {
            continue;
        }
        // Uma linha pode declarar vários alvos: `build test: deps`.
        for target in name.split_whitespace() {
            if !found.iter().any(|t: &String| t == target) {
                found.push(target.to_string());
            }
        }
    }
    found
}

/// Chaves de `scripts` do `package.json`.
///
/// JSON inválido devolve lista vazia em vez de erro: o arquivo pode estar sendo
/// editado neste exato instante, e a completação não é lugar de reclamar disso.
pub fn parse_package_scripts(source: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(source) else {
        return Vec::new();
    };
    let Some(scripts) = value.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    scripts.keys().cloned().collect()
}

/// Lê o arquivo do projeto que o provedor precisa, subindo até a raiz do repo.
///
/// Sobe porque `npm run` funciona de um subdiretório: o gerenciador procura o
/// `package.json` mais próximo, e a completação que só olhasse o cwd calaria
/// justamente onde o comando funciona.
pub fn find_upwards(
    start: &Path,
    name: &str,
    stop_at: Option<&Path>,
) -> Option<std::path::PathBuf> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        let candidate = current.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if stop_at.is_some_and(|root| current == root) {
            return None;
        }
        dir = current.parent();
    }
    None
}

/// Ordena os candidatos do provedor pelo que o dono de fato usa.
///
/// Esta é a inversão que a entrega faz: o histórico deixa de decidir **quais**
/// candidatos existem e passa a decidir **em que ordem** eles aparecem. O que
/// nunca foi usado continua na lista — some seria voltar ao problema de origem,
/// onde a máquina nova não completa nada.
///
/// `used` chega ordenado por recência (é o que `next_tokens` devolve).
pub fn rank(candidates: Vec<String>, used: &[String]) -> Vec<String> {
    let position = |name: &String| used.iter().position(|u| u == name);
    let mut ranked = candidates;
    ranked.sort_by(|a, b| match (position(a), position(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.cmp(b),
    });
    ranked
}

/// Quantas vezes cada próximo token já rodou depois de `prefix`.
///
/// `rows` é `(comando, vezes)` do histórico. `git commit -m a` e `git commit
/// --amend` somam os dois em `commit` quando o prefixo é `git `.
pub fn token_uses(
    rows: &[(String, u32)],
    prefix: &str,
    token: &str,
) -> std::collections::HashMap<String, u32> {
    let mut uses = std::collections::HashMap::new();
    for (command, count) in rows {
        let Some(next) = command
            .strip_prefix(prefix)
            .and_then(|rest| rest.split_whitespace().next())
        else {
            continue;
        };
        if next.starts_with(token) && next != token {
            *uses.entry(next.to_string()).or_insert(0) += count;
        }
    }
    uses
}

/// Uma linha da lista: o valor, o que ele faz e de onde veio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub value: String,
    pub description: Option<String>,
    pub uses: u32,
    /// `subcommand` ou `option` quando a base conhece; senão `None`.
    pub kind: Option<String>,
    /// A forma longa da mesma flag, quando as duas vieram: `-m` + `--message`.
    pub alias: Option<String>,
}

/// Junta a flag curta e a longa numa linha só (`-m, --message`).
///
/// O par é reconhecido pela descrição igual, que é como a base registra as
/// duas formas da mesma flag. A curta fica como valor (é a que se digita), a
/// longa vira `alias`, e o uso das duas soma.
pub fn pair_flags(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let is_short = |v: &str| v.len() == 2 && v.starts_with('-') && !v.starts_with("--");
    let is_option = |c: &Candidate| c.kind.as_deref() == Some("option");
    // Primeiro os pares, depois a saída: a longa pode vir antes da curta na
    // lista, e ela some de onde estiver.
    let mut partner: Vec<Option<usize>> = vec![None; candidates.len()];
    let mut absorbed = vec![false; candidates.len()];
    for (i, short) in candidates.iter().enumerate() {
        if !is_short(&short.value) || !is_option(short) || short.description.is_none() {
            continue;
        }
        let found = candidates.iter().enumerate().position(|(j, long)| {
            !absorbed[j]
                && long.value.starts_with("--")
                && is_option(long)
                && long.description == short.description
        });
        if let Some(j) = found {
            absorbed[j] = true;
            partner[i] = Some(j);
        }
    }
    candidates
        .iter()
        .enumerate()
        .filter(|(i, _)| !absorbed[*i])
        .map(|(i, c)| {
            let mut item = c.clone();
            if let Some(j) = partner[i] {
                item.alias = Some(candidates[j].value.clone());
                item.uses += candidates[j].uses;
            }
            item
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(value: &str, desc: Option<&str>, kind: Option<&str>, uses: u32) -> Candidate {
        Candidate {
            value: value.to_string(),
            description: desc.map(str::to_string),
            uses,
            kind: kind.map(str::to_string),
            alias: None,
        }
    }

    #[test]
    fn uso_soma_por_proximo_token() {
        let rows = vec![
            ("git commit -m a".to_string(), 3),
            ("git commit --amend".to_string(), 2),
            ("git status".to_string(), 5),
            ("git".to_string(), 9),
        ];
        let uses = token_uses(&rows, "git ", "");
        assert_eq!(uses.get("commit"), Some(&5));
        assert_eq!(uses.get("status"), Some(&5));
        assert_eq!(uses.len(), 2);
        assert!(!token_uses(&rows, "git ", "st").contains_key("commit"));
    }

    #[test]
    fn flag_curta_e_longa_viram_uma_linha() {
        let msg = Some("Use the given message");
        let out = pair_flags(vec![
            cand("--message", msg, Some("option"), 1),
            cand("-m", msg, Some("option"), 4),
            cand("--amend", Some("Replace the tip"), Some("option"), 0),
            cand("main", None, None, 0),
        ]);
        let values: Vec<&str> = out.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["-m", "--amend", "main"]);
        assert_eq!(out[0].alias.as_deref(), Some("--message"));
        assert_eq!(out[0].uses, 5);
    }

    #[test]
    fn descricao_diferente_nao_junta() {
        let out = pair_flags(vec![
            cand("-a", Some("all"), Some("option"), 0),
            cand("--amend", Some("amend"), Some("option"), 0),
        ]);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| c.alias.is_none()));
    }

    #[test]
    fn a_tabela_reconhece_o_subcomando_e_nao_so_o_binario() {
        assert_eq!(provider_for("git checkout "), Some(Provider::GitBranch));
        assert_eq!(provider_for("git commit "), None);
        assert_eq!(provider_for("npm run "), Some(Provider::NpmScript));
        assert_eq!(provider_for("npm install "), None);
    }

    #[test]
    fn sem_espaco_no_fim_ainda_se_completa_o_subcomando() {
        // `git checkout` sem espaço é o próprio subcomando sendo digitado —
        // oferecer branch ali atropelaria `git checkout-index`.
        assert_eq!(provider_for("git checkout"), None);
        assert_eq!(provider_for("git checkout "), Some(Provider::GitBranch));
    }

    #[test]
    fn flag_no_meio_nao_desliga_o_provedor() {
        assert_eq!(
            provider_for("git --no-pager checkout "),
            Some(Provider::GitBranch)
        );
    }

    #[test]
    fn valor_de_flag_separado_por_espaco_desliga_o_provedor() {
        // Limitação conhecida e aceita: `core.quotePath=false` não começa com
        // `-`, vira palavra e desloca a contagem. Saber quais flags consomem o
        // próximo argumento exigiria uma tabela por binário — e o custo de
        // errar para o outro lado (oferecer branch onde não cabe) é maior que
        // cair no histórico. `git -c X checkout` também não é coisa que se
        // digite à mão; é forma de script.
        assert_eq!(provider_for("git -c core.quotePath=false checkout "), None);
    }

    #[test]
    fn caminho_do_binario_nao_atrapalha() {
        assert_eq!(
            provider_for("/usr/bin/git checkout "),
            Some(Provider::GitBranch)
        );
    }

    #[test]
    fn git_branch_so_oferece_branch_quando_a_flag_age_sobre_uma_existente() {
        // `git branch <nome>` está CRIANDO: sugerir um nome existente ali é
        // sugerir um erro, porque o git recusa.
        assert_eq!(provider_for("git branch "), None);
        assert_eq!(provider_for("git branch -d "), Some(Provider::GitBranch));
        assert_eq!(provider_for("git branch -D "), Some(Provider::GitBranch));
    }

    #[test]
    fn push_oferece_remoto_e_depois_branch() {
        assert_eq!(provider_for("git push "), Some(Provider::GitRemote));
        assert_eq!(provider_for("git push origin "), Some(Provider::GitBranch));
        assert_eq!(provider_for("git pull -r "), Some(Provider::GitRemote));
        assert_eq!(provider_for("git fetch origin main "), None);
    }

    #[test]
    fn remote_so_oferece_remoto_onde_ele_ja_existe() {
        assert_eq!(
            provider_for("git remote remove "),
            Some(Provider::GitRemote)
        );
        assert_eq!(provider_for("git remote show "), Some(Provider::GitRemote));
        // `remote add` cria: o nome é novo, e o que existe seria colisão.
        assert_eq!(provider_for("git remote add "), None);
        assert_eq!(provider_for("git remote "), None);
    }

    #[test]
    fn add_e_restore_oferecem_o_que_mudou() {
        assert_eq!(provider_for("git add "), Some(Provider::GitChangedFile));
        assert_eq!(
            provider_for("git add src/a.rs "),
            Some(Provider::GitChangedFile)
        );
        assert_eq!(
            provider_for("git restore --staged "),
            Some(Provider::GitChangedFile)
        );
        assert_eq!(provider_for("git commit "), None);
    }

    #[test]
    fn ssh_so_completa_host_na_primeira_posicao() {
        assert_eq!(provider_for("ssh "), Some(Provider::SshHost));
        // Depois do host vem comando remoto, não outro host.
        assert_eq!(provider_for("ssh servidor "), None);
    }

    #[test]
    fn make_completa_alvo_em_qualquer_posicao() {
        // `make clean build` é legítimo, então o provedor continua valendo.
        assert_eq!(provider_for("make "), Some(Provider::MakeTarget));
        assert_eq!(provider_for("make clean "), Some(Provider::MakeTarget));
    }

    #[test]
    fn docker_completa_container_so_em_subcomando_que_age_sobre_um_existente() {
        assert_eq!(
            provider_for("docker exec "),
            Some(Provider::DockerContainer)
        );
        assert_eq!(
            provider_for("docker logs "),
            Some(Provider::DockerContainer)
        );
        // `docker run` CRIA: oferecer nome existente é sugerir colisão.
        assert_eq!(provider_for("docker run "), None);
        assert_eq!(provider_for("docker build "), None);
        assert_eq!(provider_for("docker ps "), None);
    }

    #[test]
    fn linha_vazia_nao_tem_provedor() {
        assert_eq!(provider_for(""), None);
        assert_eq!(provider_for("   "), None);
    }

    #[test]
    fn alvos_do_makefile_saem_na_ordem_do_arquivo() {
        let makefile = "\
CC := gcc
.PHONY: all clean

all: build test
\t@echo pronto

build:
\tcargo build

test: build
\tcargo test
";
        assert_eq!(
            parse_make_targets(makefile),
            vec!["all", "build", "test"],
            "`:=` é atribuição e `.PHONY` começa com ponto — nenhum dos dois é alvo"
        );
    }

    #[test]
    fn uma_linha_pode_declarar_varios_alvos() {
        assert_eq!(
            parse_make_targets("build test: deps\n\techo x\n"),
            vec!["build", "test"]
        );
    }

    #[test]
    fn alvo_com_variavel_no_nome_fica_de_fora() {
        // Expandir `$(BIN)` exigiria interpretar o Makefile, e um alvo
        // inventado é pior que um alvo faltando.
        assert!(parse_make_targets("$(BIN): main.o\n\tld -o $@\n").is_empty());
    }

    #[test]
    fn receita_com_dois_pontos_nao_vira_alvo() {
        // A linha da receita começa com TAB. Sem essa guarda, `echo a:b` viraria
        // um alvo chamado `echo a`.
        assert_eq!(parse_make_targets("run:\n\techo a:b\n"), vec!["run"]);
    }

    #[test]
    fn scripts_do_package_json() {
        let mut found = parse_package_scripts(
            r#"{"name":"x","scripts":{"dev":"vite","build":"tsc"},"devDependencies":{}}"#,
        );
        found.sort();
        assert_eq!(found, vec!["build", "dev"]);
    }

    #[test]
    fn package_json_quebrado_devolve_vazio_em_vez_de_erro() {
        // O arquivo pode estar sendo editado neste exato instante.
        assert!(parse_package_scripts("{\"scripts\": {").is_empty());
        assert!(parse_package_scripts("").is_empty());
        assert!(parse_package_scripts("{\"name\":\"x\"}").is_empty());
    }

    #[test]
    fn o_historico_ordena_mas_nao_exclui() {
        // O ponto da entrega: `deploy` nunca foi usado e continua na lista. Se
        // sumisse, a máquina nova voltaria a não completar nada.
        let candidatos = vec!["build".into(), "deploy".into(), "test".into()];
        let usados = vec!["test".to_string(), "build".to_string()];

        assert_eq!(
            rank(candidatos, &usados),
            vec!["test", "build", "deploy"],
            "usados primeiro na ordem de uso, o resto em ordem alfabética"
        );
    }

    #[test]
    fn sem_historico_a_ordem_e_alfabetica_e_estavel() {
        let candidatos = vec!["test".into(), "build".into(), "deploy".into()];
        assert_eq!(rank(candidatos, &[]), vec!["build", "deploy", "test"]);
    }

    #[test]
    fn find_upwards_sobe_ate_a_raiz_e_para_nela() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();

        // `npm run` funciona de um subdiretório, então a completação também tem
        // de funcionar de lá.
        assert_eq!(
            find_upwards(&nested, "package.json", Some(root)),
            Some(root.join("package.json"))
        );
        // E não escapa da raiz declarada.
        assert_eq!(find_upwards(&nested, "Makefile", Some(root)), None);
    }
}

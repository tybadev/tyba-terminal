//! Leitor de "regra ampla" nas configurações de permissão do Claude Code.
//!
//! No fluxo híbrido o amarelo do Claude é decidido pelo prompt nativo dele, e
//! o hook do TYBA sai sem opinar. Isso só é seguro enquanto o Claude ainda
//! **pergunta** alguma coisa: uma configuração que deixa o amarelo passar
//! calado (allow de `Bash` inteiro, modo que dispensa aprovação) seria um
//! "modo sem aprovações" — proibido pelo `SECURITY.md`. Esta função é o que o
//! core consulta para saber se precisa forçar a pergunta (`ask`).
//!
//! Regra específica (`Bash(cargo test:*)`) **não** é ampla: é o "don't ask
//! again" funcionando. Tratá-la como ampla faria o Claude perguntar sempre —
//! exatamente a queixa que originou o fluxo híbrido.
//!
//! Só leitura, sem estado e sem cache: o core relê a cada `PreToolUse`
//! amarelo, para que uma mudança de config do usuário valha na hora.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Entradas de `permissions.allow` que liberam **todo** comando Bash. Qualquer
/// outra forma `Bash(...)` restringe a um prefixo e é escolha legítima do
/// usuário.
const BROAD_BASH_RULES: &[&str] = &["Bash", "Bash(*)", "Bash(:*)", "Bash(**)"];

/// Modos em que o Claude não pergunta nada — com eles, "o Claude decide o
/// amarelo" vira "ninguém decide".
const SILENT_MODES: &[&str] = &["bypassPermissions", "dontAsk"];

/// Arquivos de configuração que o Claude lê dentro de um diretório `.claude`.
const SETTINGS_FILES: &[&str] = &["settings.json", "settings.local.json"];

/// Diz se alguma configuração do Claude que a sessão do agente vai carregar
/// deixa o amarelo sem pergunta.
///
/// `home` é o `HOME` **do agente**, nunca o `CLAUDE_CONFIG_DIR` do app: essa
/// variável está na denylist de env, então o Claude do agente sempre usa
/// `$HOME/.claude` — ler outro lugar seria olhar a config errada.
///
/// `worktree` e `repo_root` entram os dois porque o Claude da sessão roda no
/// worktree, mas o repo principal também pode ter `.claude/` versionado ou
/// local. Quando são o mesmo diretório, lê-se duas vezes o mesmo arquivo —
/// barato e inofensivo.
pub fn claude_settings_are_broad(home: &Path, worktree: &Path, repo_root: &Path) -> bool {
    settings_are_broad_with_managed(home, worktree, repo_root, managed_settings_dir().as_deref())
}

/// Núcleo com o diretório de managed settings injetável: o caminho real é do
/// sistema (`/Library/...`, `/etc/...`) e o teste não pode escrever lá.
fn settings_are_broad_with_managed(
    home: &Path,
    worktree: &Path,
    repo_root: &Path,
    managed_dir: Option<&Path>,
) -> bool {
    candidate_files(home, worktree, repo_root, managed_dir)
        .iter()
        .any(|path| file_is_broad(path))
}

/// Todos os arquivos que contam, em qualquer ordem: basta **um** amplo para
/// forçar a pergunta, então precedência entre escopos não importa aqui. Um
/// `deny` num escopo mais forte poderia, em tese, anular o allow amplo — mas
/// errar para o lado de perguntar é o erro barato.
fn candidate_files(
    home: &Path,
    worktree: &Path,
    repo_root: &Path,
    managed_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut files = Vec::new();
    // Usuário, projeto no worktree, projeto na raiz do repo principal.
    for base in [home, worktree, repo_root] {
        let dot_claude = base.join(".claude");
        for name in SETTINGS_FILES {
            files.push(dot_claude.join(name));
        }
    }
    // Gerenciado pelo SO: o arquivo principal e os drop-ins de
    // `managed-settings.d/` (o Claude lê os dois; um admin que liga
    // `bypassPermissions` por drop-in silencia o amarelo do mesmo jeito).
    if let Some(dir) = managed_dir {
        files.push(dir.join("managed-settings.json"));
        if let Ok(entries) = std::fs::read_dir(dir.join("managed-settings.d")) {
            let mut drop_ins: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
                .collect();
            drop_ins.sort();
            files.extend(drop_ins);
        }
    }
    files
}

/// Diretório de managed settings do Claude Code por SO, conferido no binário
/// instalado (2.1.281): o Claude junta `managed-settings.json` a este
/// diretório e lê os drop-ins em `managed-settings.d/` dentro dele.
fn managed_settings_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(PathBuf::from("/Library/Application Support/ClaudeCode"))
    } else if cfg!(target_os = "windows") {
        Some(PathBuf::from(r"C:\Program Files\ClaudeCode"))
    } else if cfg!(target_os = "linux") {
        Some(PathBuf::from("/etc/claude-code"))
    } else {
        None
    }
}

/// Arquivo ausente, ilegível ou com JSON inválido não é amplo: o próprio
/// Claude ignora esses arquivos, então eles não silenciam nada.
fn file_is_broad(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    settings_value_is_broad(&value)
}

fn settings_value_is_broad(value: &Value) -> bool {
    let Some(permissions) = value.get("permissions") else {
        return false;
    };
    let silent_mode = permissions
        .get("defaultMode")
        .and_then(Value::as_str)
        .is_some_and(|mode| SILENT_MODES.contains(&mode));
    let broad_allow = permissions
        .get("allow")
        .and_then(Value::as_array)
        .is_some_and(|rules| {
            rules
                .iter()
                .filter_map(Value::as_str)
                .any(|rule| BROAD_BASH_RULES.contains(&rule.trim()))
        });
    silent_mode || broad_allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    /// Três diretórios distintos (home, worktree, raiz do repo) e um de
    /// managed settings, todos vazios. Cada teste planta só o que precisa —
    /// fixtures sintéticas, nunca cópia de config real.
    struct Fixture {
        home: TempDir,
        worktree: TempDir,
        repo_root: TempDir,
        managed: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                home: TempDir::new().unwrap(),
                worktree: TempDir::new().unwrap(),
                repo_root: TempDir::new().unwrap(),
                managed: TempDir::new().unwrap(),
            }
        }

        fn is_broad(&self) -> bool {
            settings_are_broad_with_managed(
                self.home.path(),
                self.worktree.path(),
                self.repo_root.path(),
                Some(self.managed.path()),
            )
        }

        fn write_raw(path: &Path, contents: &str) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn write_dot_claude(base: &Path, file: &str, value: Value) {
            Self::write_raw(&base.join(".claude").join(file), &value.to_string());
        }
    }

    fn allow(rules: &[&str]) -> Value {
        json!({ "permissions": { "allow": rules } })
    }

    fn mode(m: &str) -> Value {
        json!({ "permissions": { "defaultMode": m } })
    }

    #[test]
    fn sem_nenhum_arquivo_nao_e_amplo() {
        assert!(!Fixture::new().is_broad());
    }

    #[test]
    fn allow_de_bash_inteiro_e_amplo_em_todas_as_grafias() {
        for rule in ["Bash", "Bash(*)", "Bash(:*)", "Bash(**)"] {
            let fx = Fixture::new();
            Fixture::write_dot_claude(fx.home.path(), "settings.json", allow(&[rule]));
            assert!(fx.is_broad(), "{rule} deveria contar como amplo");
        }
    }

    #[test]
    fn modos_sem_pergunta_sao_amplos() {
        for m in ["bypassPermissions", "dontAsk"] {
            let fx = Fixture::new();
            Fixture::write_dot_claude(fx.home.path(), "settings.json", mode(m));
            assert!(fx.is_broad(), "defaultMode {m} deveria contar como amplo");
        }
    }

    #[test]
    fn regra_especifica_nao_e_ampla() {
        // É o "don't ask again" funcionando: tratá-la como ampla faria o
        // Claude perguntar sempre.
        let fx = Fixture::new();
        Fixture::write_dot_claude(
            fx.home.path(),
            "settings.json",
            allow(&["Bash(cargo test:*)", "Bash(git status)", "Read", "Edit(*)"]),
        );
        assert!(!fx.is_broad());
    }

    #[test]
    fn modos_que_ainda_perguntam_nao_sao_amplos() {
        for m in ["default", "acceptEdits", "plan"] {
            let fx = Fixture::new();
            Fixture::write_dot_claude(fx.home.path(), "settings.json", mode(m));
            assert!(!fx.is_broad(), "defaultMode {m} não deveria ser amplo");
        }
    }

    #[test]
    fn bash_amplo_em_deny_ou_ask_nao_conta() {
        // Só o allow silencia; `Bash(*)` em deny/ask faz o contrário.
        let fx = Fixture::new();
        Fixture::write_dot_claude(
            fx.home.path(),
            "settings.json",
            json!({ "permissions": { "deny": ["Bash(*)"], "ask": ["Bash"] } }),
        );
        assert!(!fx.is_broad());
    }

    #[test]
    fn json_quebrado_nao_e_amplo() {
        let fx = Fixture::new();
        Fixture::write_raw(
            &fx.home.path().join(".claude/settings.json"),
            r#"{"permissions": {"allow": ["Bash(*)"]"#,
        );
        assert!(!fx.is_broad());
    }

    #[test]
    fn forma_inesperada_nao_e_ampla_nem_quebra() {
        let fx = Fixture::new();
        Fixture::write_dot_claude(
            fx.home.path(),
            "settings.json",
            json!({ "permissions": { "allow": "Bash(*)", "defaultMode": 1 } }),
        );
        Fixture::write_dot_claude(fx.worktree.path(), "settings.json", json!(["Bash"]));
        assert!(!fx.is_broad());
    }

    #[test]
    fn json_quebrado_num_escopo_nao_esconde_amplo_em_outro() {
        let fx = Fixture::new();
        Fixture::write_raw(&fx.home.path().join(".claude/settings.json"), "{");
        Fixture::write_dot_claude(fx.repo_root.path(), "settings.json", allow(&["Bash"]));
        assert!(fx.is_broad());
    }

    // Cada escopo detectado isoladamente: se a leitura de um deles sumir,
    // o teste correspondente reprova.

    #[test]
    fn escopo_usuario_settings_detectado() {
        let fx = Fixture::new();
        Fixture::write_dot_claude(fx.home.path(), "settings.json", allow(&["Bash(*)"]));
        assert!(fx.is_broad());
    }

    #[test]
    fn escopo_usuario_local_detectado() {
        let fx = Fixture::new();
        Fixture::write_dot_claude(fx.home.path(), "settings.local.json", allow(&["Bash(*)"]));
        assert!(fx.is_broad());
    }

    #[test]
    fn escopo_projeto_no_worktree_detectado() {
        // O ataque de security.md: o agente planta `Bash(*)` no próprio
        // worktree, que está dentro do `writable_root` dele.
        let fx = Fixture::new();
        Fixture::write_dot_claude(fx.worktree.path(), "settings.json", allow(&["Bash(*)"]));
        assert!(fx.is_broad());
    }

    #[test]
    fn escopo_local_no_worktree_detectado() {
        let fx = Fixture::new();
        Fixture::write_dot_claude(
            fx.worktree.path(),
            "settings.local.json",
            mode("bypassPermissions"),
        );
        assert!(fx.is_broad());
    }

    #[test]
    fn escopo_projeto_na_raiz_do_repo_detectado() {
        let fx = Fixture::new();
        Fixture::write_dot_claude(fx.repo_root.path(), "settings.json", mode("dontAsk"));
        assert!(fx.is_broad());
    }

    #[test]
    fn escopo_local_na_raiz_do_repo_detectado() {
        let fx = Fixture::new();
        Fixture::write_dot_claude(fx.repo_root.path(), "settings.local.json", allow(&["Bash"]));
        assert!(fx.is_broad());
    }

    #[test]
    fn escopo_gerenciado_detectado() {
        let fx = Fixture::new();
        Fixture::write_raw(
            &fx.managed.path().join("managed-settings.json"),
            &mode("bypassPermissions").to_string(),
        );
        assert!(fx.is_broad());
    }

    #[test]
    fn drop_in_gerenciado_detectado() {
        let fx = Fixture::new();
        Fixture::write_raw(
            &fx.managed.path().join("managed-settings.d/10-dev.json"),
            &allow(&["Bash(**)"]).to_string(),
        );
        assert!(fx.is_broad());
    }

    #[test]
    fn drop_in_gerenciado_sem_extensao_json_e_ignorado() {
        let fx = Fixture::new();
        Fixture::write_raw(
            &fx.managed.path().join("managed-settings.d/10-dev.json.bak"),
            &allow(&["Bash(**)"]).to_string(),
        );
        assert!(!fx.is_broad());
    }

    #[test]
    fn sem_diretorio_gerenciado_le_os_demais_escopos() {
        let fx = Fixture::new();
        assert!(!settings_are_broad_with_managed(
            fx.home.path(),
            fx.worktree.path(),
            fx.repo_root.path(),
            None,
        ));
        Fixture::write_dot_claude(fx.home.path(), "settings.json", allow(&["Bash"]));
        assert!(settings_are_broad_with_managed(
            fx.home.path(),
            fx.worktree.path(),
            fx.repo_root.path(),
            None,
        ));
    }

    #[test]
    fn config_fora_de_dot_claude_nao_conta() {
        // O Claude lê `<base>/.claude/settings*.json`, não `<base>/settings.json`.
        let fx = Fixture::new();
        Fixture::write_raw(
            &fx.home.path().join("settings.json"),
            &allow(&["Bash(*)"]).to_string(),
        );
        assert!(!fx.is_broad());
    }
}

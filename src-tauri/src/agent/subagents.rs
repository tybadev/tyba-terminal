use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Runtime};

use crate::approvals::now_ms;
use crate::session::redact::redact;
use crate::session::SessionId;
use crate::status::subagent_transcript::{TranscriptWatchers, WatchSpec};
use crate::status::transcript::clean_summary;

pub type SharedSubagents = Arc<SubagentTracker>;

/// Fecha o turno da sessão em `Idle` quando o último subagente async termina fora
/// do fluxo de hooks. Recebe a sessão e o resumo do turno do orquestrador.
pub type IdleCoordinator = Arc<dyn Fn(SessionId, Option<String>) + Send + Sync>;

pub(crate) type SharedSessions = Arc<Mutex<HashMap<SessionId, SessionSubagents>>>;
pub(crate) type SharedCoordinator = Arc<Mutex<Option<IdleCoordinator>>>;

pub const EVENT_SUBAGENTS_CHANGED: &str = "subagents://changed";

const SUMMARY_MAX_CHARS: usize = 280;

const MIN_SUBAGENT_ID_LEN: usize = 8;

/// Quantos subagentes concluídos o painel guarda por sessão. O histórico
/// completo vive no transcript; o painel só precisa do recente.
const MAX_SETTLED_RUNS: usize = 40;

/// Quantas ações anteriores o painel mostra embaixo do AGORA.
const MAX_RECENT_ACTIVITY: usize = 2;

/// Um `agent_id` real de subagente do Claude Code é hexadecimal (17 chars nesta
/// máquina). Item de fila / prompt enfileirado não carrega um id assim — o gate
/// evita transformar stop espúrio em entrada fantasma no painel.
pub(crate) fn is_plausible_subagent_id(agent_id: &str) -> bool {
    agent_id.len() >= MIN_SUBAGENT_ID_LEN && agent_id.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Starting,
    Running,
    Done,
}

#[derive(Debug, Clone)]
pub struct SubagentRun {
    pub agent_id: Option<String>,
    pub agent_type: String,
    pub description: String,
    pub status: SubagentStatus,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub summary: Option<String>,
    pub interrupted: bool,
    /// Acabou parado (`<status>blocked</status>` na fila do pai): não
    /// concluiu nem foi interrompido — está esperando algo e não vai seguir
    /// sozinho. Mostrado como "parado".
    pub blocked: bool,
    /// Rodada a que o nó pertence (1, 2, …). O painel agrupa por ela: com
    /// descrições iguais entre rodadas, a lista plana parecia duplicada.
    pub round: u32,
    pub transcript_path: Option<PathBuf>,
    pub parent_transcript_path: Option<PathBuf>,
    /// O que o subagente está fazendo AGORA: a última ferramenta que ele
    /// chamou (`PreToolUse` com `agent_id`). Some quando ele termina — aí quem
    /// fala é o `summary`.
    pub activity: Option<ToolActivity>,
    /// A descrição veio do `meta.json` do próprio subagente (e não do palpite
    /// por ordem): não precisa mais ser conferida.
    pub description_confirmed: bool,
    /// Quando a `activity` atual começou (ms), para o "há Ns" do painel.
    pub activity_at_ms: Option<u64>,
    /// As ações anteriores, da mais nova para a mais antiga (até
    /// [`MAX_RECENT_ACTIVITY`]). O painel mostra embaixo do AGORA.
    pub recent: Vec<(ToolActivity, u64)>,
    /// Tamanho do sidecar quando o subagente foi acordado (Done→Running). O
    /// fim que já estava no arquivo é o da volta anterior: só conta fim por
    /// arquivo depois que ele crescer além disto.
    pub stale_end_len: Option<u64>,
}

/// Ferramenta + argumento curto, já recortados para uma linha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolActivity {
    pub tool: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubagentRunDto {
    pub agent_id: Option<String>,
    pub agent_type: String,
    pub description: String,
    pub status: SubagentStatus,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub summary: Option<String>,
    pub interrupted: bool,
    pub blocked: bool,
    pub round: u32,
    /// Ferramenta em curso (`Bash`, `Read`…); `None` sem atividade ou já
    /// terminado.
    pub activity_tool: Option<String>,
    /// O argumento curto dela (comando, caminho, padrão), redigido.
    pub activity_detail: Option<String>,
    /// Quando a ação em curso começou (ms).
    pub activity_at_ms: Option<u64>,
    /// Ações anteriores, mais nova primeiro, redigidas; vazio se terminado.
    pub recent_activity: Vec<RecentActivityDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentActivityDto {
    pub tool: String,
    pub detail: Option<String>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubagentSnapshot {
    pub focused: Option<String>,
    pub subagents: Vec<SubagentRunDto>,
}

#[derive(Clone, Serialize)]
struct SubagentsChangedPayload {
    session_id: SessionId,
    focused: Option<String>,
    subagents: Vec<SubagentRunDto>,
}

impl SubagentRunDto {
    fn redacted(run: &SubagentRun) -> Self {
        Self {
            agent_id: run.agent_id.clone(),
            agent_type: redact(&run.agent_type).into_owned(),
            description: redact(&run.description).into_owned(),
            status: run.status,
            started_at_ms: run.started_at_ms,
            ended_at_ms: run.ended_at_ms,
            summary: run.summary.as_deref().map(|s| redact(s).into_owned()),
            interrupted: run.interrupted,
            blocked: run.blocked,
            round: run.round,
            activity_tool: live_activity(run).map(|a| a.tool.clone()),
            activity_detail: live_activity(run)
                .and_then(|a| a.detail.as_deref())
                .map(|d| redact(d).into_owned()),
            activity_at_ms: live_activity(run).and(run.activity_at_ms),
            recent_activity: if run.status == SubagentStatus::Done {
                Vec::new()
            } else {
                run.recent
                    .iter()
                    .map(|(a, at_ms)| RecentActivityDto {
                        tool: a.tool.clone(),
                        detail: a.detail.as_deref().map(|d| redact(d).into_owned()),
                        at_ms: *at_ms,
                    })
                    .collect()
            },
        }
    }
}

/// Atividade só de quem ainda roda: terminado, a linha mostra o resultado.
fn live_activity(run: &SubagentRun) -> Option<&ToolActivity> {
    (run.status != SubagentStatus::Done)
        .then_some(run.activity.as_ref())
        .flatten()
}

/// Teto do argumento mostrado: a linha do painel tem ~40 colunas, e o texto
/// inteiro de um heredoc não cabe nem é lido ali.
const ACTIVITY_MAX_CHARS: usize = 120;

/// A atividade de uma chamada de ferramenta, numa linha.
///
/// `fallback` é a descrição normalizada do gate (`NormalizedToolUse`), que já
/// traz o argumento das ferramentas de escrita e de rede. As de leitura saem
/// dela só com o nome — aqui se pega o campo que diz O QUE se lê.
pub(crate) fn tool_activity(tool: &str, input: Option<&Value>, fallback: &str) -> ToolActivity {
    let field = |key: &str| input.and_then(|v| v.get(key)).and_then(Value::as_str);
    let detail = match tool {
        "Bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" => field("file_path"),
        "NotebookRead" | "NotebookEdit" => field("notebook_path"),
        "Grep" | "Glob" => field("pattern"),
        "LS" => field("path"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        _ => fallback
            .strip_prefix(tool)
            .map(str::trim)
            .or(Some(fallback)),
    };
    let detail = detail
        .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
        .filter(|d| *d != tool)
        .map(|d| {
            if d.chars().count() > ACTIVITY_MAX_CHARS {
                let cut: String = d.chars().take(ACTIVITY_MAX_CHARS).collect();
                format!("{cut}…")
            } else {
                d.to_string()
            }
        });
    ToolActivity {
        tool: tool.to_string(),
        detail,
    }
}

/// Como um subagente acabou, pelo sinal de disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Completed,
    /// `failed`/`killed`, ou morto sem resposta final.
    Interrupted,
    /// `blocked`: parou esperando algo; não segue sozinho.
    Blocked,
}

pub struct Coordination {
    pub viewer_disarmed: bool,
    pub panel_disarmed: bool,
}

#[derive(Default)]
pub(crate) struct SessionSubagents {
    runs: Vec<SubagentRun>,
    focused: Option<String>,
    coordinated: bool,
    viewer_disarmed: bool,
    panel_disarmed: bool,
    orchestrator_idle: bool,
    orchestrator_idle_summary: Option<String>,
    /// Rodada corrente. 0 antes do primeiro subagente.
    round: u32,
}

impl SessionSubagents {
    /// Rodada nova (primeiro subagente chegando com TODOS os anteriores Done)
    /// re-arma a coordenação: o painel volta a auto-abrir mesmo que o usuário
    /// o tenha fechado na rodada anterior. Fechar é dispensar ESTA rodada, não
    /// todas as próximas.
    ///
    /// Os nós das rodadas anteriores FICAM: o painel é a árvore da sessão, com
    /// a rodada que roda no topo e as concluídas embaixo (A5.3). Limpar a cada
    /// rodada escondia o que acabou de acontecer, e a ordem do snapshot já
    /// separa o que roda do que terminou. O teto `MAX_SETTLED_RUNS` impede a
    /// lista de crescer sem fim numa sessão longa.
    ///
    /// "Assentada" olha só a rodada corrente: algum nó dela já terminou e
    /// nenhum está `Running`. Um `Starting` não segura a rodada: é placeholder
    /// de spawn que pode nunca virar `Running` (spawn aprovado cujo
    /// `SubagentStart` não chegou), e exigir `Done` de todos deixava o
    /// `panel_disarmed` grudado para sempre — fechou o painel uma vez, nunca
    /// mais abria sozinho. Exigir um `Done` da rodada corrente é o que impede
    /// o próprio placeholder da rodada nova (`push_pending` → `promote`) de
    /// abrir outra rodada.
    fn rearm_for_new_round(&mut self) {
        let current = self.runs.iter().filter(|run| run.round == self.round);
        let (mut any_done, mut any_running) = (false, false);
        for run in current {
            any_done |= run.status == SubagentStatus::Done;
            any_running |= run.status == SubagentStatus::Running;
        }
        let round_settled = any_done && !any_running;
        if self.round == 0 || round_settled {
            self.round += 1;
        }
        if round_settled {
            self.focused = None;
            self.coordinated = false;
            self.viewer_disarmed = false;
            self.panel_disarmed = false;
            self.orchestrator_idle = false;
            self.orchestrator_idle_summary = None;
            self.trim_settled();
        }
    }

    /// Descarta os `Done` mais antigos além do teto. Nunca toca em ativo.
    fn trim_settled(&mut self) {
        let settled = self
            .runs
            .iter()
            .filter(|run| run.status == SubagentStatus::Done)
            .count();
        let mut excess = settled.saturating_sub(MAX_SETTLED_RUNS);
        if excess == 0 {
            return;
        }
        let mut oldest: Vec<(u64, usize)> = self
            .runs
            .iter()
            .enumerate()
            .filter(|(_, run)| run.status == SubagentStatus::Done)
            .map(|(i, run)| (run.started_at_ms, i))
            .collect();
        oldest.sort();
        let mut drop: Vec<usize> = Vec::new();
        for (_, index) in oldest {
            if excess == 0 {
                break;
            }
            drop.push(index);
            excess -= 1;
        }
        drop.sort_unstable_by(|a, b| b.cmp(a));
        for index in drop {
            self.runs.remove(index);
        }
    }

    fn push_pending(&mut self, agent_type: Option<String>, description: Option<String>, now: u64) {
        self.rearm_for_new_round();
        self.runs.push(SubagentRun {
            agent_id: None,
            agent_type: agent_type.unwrap_or_default(),
            description: description.unwrap_or_default(),
            status: SubagentStatus::Starting,
            started_at_ms: now,
            ended_at_ms: None,
            summary: None,
            interrupted: false,
            blocked: false,
            round: self.round.max(1),
            transcript_path: None,
            parent_transcript_path: None,
            activity: None,
            activity_at_ms: None,
            description_confirmed: false,
            recent: Vec::new(),
            stale_end_len: None,
        });
    }

    fn pending_hint(&self, agent_type: &str) -> Option<String> {
        self.runs
            .iter()
            .find(|run| run.status == SubagentStatus::Starting && run.agent_type == agent_type)
            .map(|run| run.description.clone())
            .filter(|description| !description.is_empty())
    }

    fn promote(
        &mut self,
        agent_id: String,
        agent_type: String,
        description: Option<String>,
        transcript_path: Option<PathBuf>,
        parent_transcript_path: Option<&Path>,
        now: u64,
    ) {
        if self.reopen(&agent_id, transcript_path.clone(), parent_transcript_path) {
            return;
        }
        self.rearm_for_new_round();
        // O tipo exato primeiro; senão, um `Agent` pedido sem `subagent_type`
        // (o Claude escolhe o tipo, e o `SubagentStart` diz qual).
        let by_description = description.as_deref().and_then(|wanted| {
            self.runs.iter().position(|run| {
                run.status == SubagentStatus::Starting
                    && (run.agent_type == agent_type || run.agent_type.is_empty())
                    && run.description == wanted
            })
        });
        let slot = by_description
            .or_else(|| {
                self.runs.iter().position(|run| {
                    run.status == SubagentStatus::Starting && run.agent_type == agent_type
                })
            })
            .or_else(|| {
                self.runs.iter().position(|run| {
                    run.status == SubagentStatus::Starting && run.agent_type.is_empty()
                })
            });
        match slot {
            Some(index) => {
                let run = &mut self.runs[index];
                run.agent_id = Some(agent_id.clone());
                run.agent_type = agent_type;
                if let Some(real) = description.as_deref().filter(|d| !d.trim().is_empty()) {
                    run.description = real.to_owned();
                }
                run.status = SubagentStatus::Running;
                run.transcript_path = transcript_path;
                run.parent_transcript_path = parent_transcript_path.map(Path::to_path_buf);
            }
            None => {
                self.runs.push(SubagentRun {
                    agent_id: Some(agent_id.clone()),
                    agent_type,
                    description: description.unwrap_or_default(),
                    status: SubagentStatus::Running,
                    started_at_ms: now,
                    ended_at_ms: None,
                    summary: None,
                    interrupted: false,
                    blocked: false,
                    round: self.round.max(1),
                    transcript_path,
                    parent_transcript_path: parent_transcript_path.map(Path::to_path_buf),
                    activity: None,
                    activity_at_ms: None,
                    description_confirmed: false,
                    recent: Vec::new(),
                    stale_end_len: None,
                });
            }
        }
        if self.focused.is_none() {
            self.focused = Some(agent_id);
        }
    }

    /// Subagente que já tem entrada e começa de novo — no modo background ele
    /// encerra o turno e é acordado pela própria tarefa de shell (`SubagentStart`
    /// repetido com o mesmo id). Volta a `Running` na MESMA entrada, sem abrir
    /// rodada: é o mesmo trabalho continuando. `false` quando o id é novo.
    fn reopen(
        &mut self,
        agent_id: &str,
        transcript_path: Option<PathBuf>,
        parent_transcript_path: Option<&Path>,
    ) -> bool {
        let Some(run) = self
            .runs
            .iter_mut()
            .find(|run| run.agent_id.as_deref() == Some(agent_id))
        else {
            return false;
        };
        if run.transcript_path.is_none() {
            run.transcript_path = transcript_path;
        }
        if run.parent_transcript_path.is_none() {
            run.parent_transcript_path = parent_transcript_path.map(Path::to_path_buf);
        }
        if run.status == SubagentStatus::Done {
            run.status = SubagentStatus::Running;
            run.ended_at_ms = None;
            run.summary = None;
            run.interrupted = false;
            run.blocked = false;
            run.activity = None;
            run.activity_at_ms = None;
            run.recent.clear();
            // O sidecar ainda termina no fim da volta anterior: sem isto, o
            // watcher o leria de novo e fecharia o subagente recém-acordado.
            run.stale_end_len = run
                .transcript_path
                .as_deref()
                .and_then(|path| std::fs::metadata(path).ok())
                .map(|meta| meta.len());
        }
        true
    }

    /// Descarta os pedidos de spawn que nenhum `SubagentStart` confirmou. Um
    /// placeholder vive só dentro do turno que o pediu: no fim do turno, o
    /// subagente de verdade já começou (o lançamento em background responde
    /// `async_launched` depois do start). Devolve quantos saíram.
    fn drop_unconfirmed(&mut self) -> usize {
        let before = self.runs.len();
        self.runs
            .retain(|run| run.status != SubagentStatus::Starting);
        before - self.runs.len()
    }

    fn remove_pending(&mut self, agent_type: Option<String>, description: Option<String>) {
        let wanted_type = agent_type.unwrap_or_default();
        let wanted_description = description.unwrap_or_default();
        let exact = (!wanted_description.is_empty())
            .then(|| {
                self.runs.iter().rposition(|run| {
                    run.status == SubagentStatus::Starting
                        && run.agent_type == wanted_type
                        && run.description == wanted_description
                })
            })
            .flatten();
        let slot = exact.or_else(|| {
            self.runs.iter().rposition(|run| {
                run.status == SubagentStatus::Starting && run.agent_type == wanted_type
            })
        });
        if let Some(index) = slot {
            self.runs.remove(index);
        }
    }

    fn mark_stopped(
        &mut self,
        agent_id: String,
        agent_type: Option<String>,
        last_assistant_message: Option<String>,
        now: u64,
    ) -> bool {
        self.refresh_description(&agent_id);
        let summary = last_assistant_message
            .as_deref()
            .and_then(|message| clean_summary(message, SUMMARY_MAX_CHARS));
        let active = self.runs.iter().position(|run| {
            run.agent_id.as_deref() == Some(agent_id.as_str()) && run.status != SubagentStatus::Done
        });
        if let Some(index) = active {
            let run = &mut self.runs[index];
            run.status = SubagentStatus::Done;
            run.ended_at_ms = Some(now);
            run.summary = summary;
            return true;
        }
        let known = self
            .runs
            .iter()
            .any(|run| run.agent_id.as_deref() == Some(agent_id.as_str()));
        if known {
            return false;
        }
        let plausible_type = agent_type
            .as_deref()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false);
        if !is_plausible_subagent_id(&agent_id) || !plausible_type {
            return false;
        }
        self.runs.push(SubagentRun {
            agent_id: Some(agent_id),
            agent_type: agent_type.unwrap_or_default(),
            description: String::new(),
            status: SubagentStatus::Done,
            started_at_ms: now,
            ended_at_ms: Some(now),
            summary,
            interrupted: false,
            blocked: false,
            round: self.round.max(1),
            transcript_path: None,
            parent_transcript_path: None,
            activity: None,
            activity_at_ms: None,
            description_confirmed: false,
            recent: Vec::new(),
            stale_end_len: None,
        });
        true
    }

    /// Guarda o que o subagente `agent_id` está fazendo. `false` quando nada
    /// mudou (subagente desconhecido, já terminado, ou a mesma atividade):
    /// aí não há o que emitir.
    fn confirm_description(&mut self, agent_id: &str) {
        if let Some(run) = self
            .runs
            .iter_mut()
            .find(|run| run.agent_id.as_deref() == Some(agent_id))
        {
            run.description_confirmed = true;
        }
    }

    /// Nome ainda do palpite por ordem: tenta o `meta.json` do próprio id de
    /// novo (no start ele pode não existir ainda). `true` se mudou.
    fn refresh_description(&mut self, agent_id: &str) -> bool {
        let Some(run) = self
            .runs
            .iter_mut()
            .find(|run| run.agent_id.as_deref() == Some(agent_id) && !run.description_confirmed)
        else {
            return false;
        };
        let Some(real) = run
            .parent_transcript_path
            .as_deref()
            .and_then(|p| meta_description_by_id(p, agent_id))
        else {
            return false;
        };
        run.description_confirmed = true;
        if run.description == real {
            return false;
        }
        run.description = real;
        true
    }

    fn note_activity(&mut self, agent_id: &str, activity: ToolActivity) -> bool {
        let renamed = self.refresh_description(agent_id);
        let Some(run) = self.runs.iter_mut().find(|run| {
            run.agent_id.as_deref() == Some(agent_id) && run.status != SubagentStatus::Done
        }) else {
            return false;
        };
        if run.activity.as_ref() == Some(&activity) {
            return renamed;
        }
        let now = now_ms();
        if let Some(prev) = run.activity.take() {
            run.recent
                .insert(0, (prev, run.activity_at_ms.unwrap_or(now)));
            run.recent.truncate(MAX_RECENT_ACTIVITY);
        }
        run.activity = Some(activity);
        run.activity_at_ms = Some(now);
        true
    }

    fn active_count(&self) -> usize {
        self.runs
            .iter()
            .filter(|run| {
                matches!(
                    run.status,
                    SubagentStatus::Starting | SubagentStatus::Running
                )
            })
            .count()
    }

    /// Marca `Done` um subagente `Running` cuja conclusão foi detectada pelo
    /// arquivo (EOF estável + última fala assistant). Não toca em `Starting`
    /// (ainda sem `agent_id`) nem em runs já `Done`.
    fn finish_running_by_file(
        &mut self,
        agent_id: &str,
        summary: Option<String>,
        now: u64,
    ) -> bool {
        let slot = self.runs.iter().position(|run| {
            run.agent_id.as_deref() == Some(agent_id) && run.status == SubagentStatus::Running
        });
        let Some(index) = slot else {
            return false;
        };
        let run = &mut self.runs[index];
        run.status = SubagentStatus::Done;
        run.ended_at_ms = Some(now);
        run.summary = summary.and_then(|s| clean_summary(&s, SUMMARY_MAX_CHARS));
        true
    }

    fn finish_running_blocked(
        &mut self,
        agent_id: &str,
        summary: Option<String>,
        now: u64,
    ) -> bool {
        if self.finish_running_by_file(agent_id, summary.clone(), now) {
            if let Some(run) = self.runs.iter_mut().find(|run| {
                run.agent_id.as_deref() == Some(agent_id) && run.status == SubagentStatus::Done
            }) {
                run.blocked = true;
            }
            return true;
        }
        // Outro caminho (hook `SubagentStop`, sidecar) pode ter fechado o run
        // antes de a fila do pai trazer o `blocked`: sem isto o "parado" se
        // perdia e o painel dizia "concluído". Run já `Done` (e não
        // interrompido) ganha a marca e mantém o `ended_at` que tinha; já
        // parado é no-op.
        let Some(run) = self.runs.iter_mut().find(|run| {
            run.agent_id.as_deref() == Some(agent_id)
                && run.status == SubagentStatus::Done
                && !run.interrupted
        }) else {
            return false;
        };
        if run.blocked {
            return false;
        }
        run.blocked = true;
        if run.summary.is_none() {
            run.summary = summary.and_then(|s| clean_summary(&s, SUMMARY_MAX_CHARS));
        }
        true
    }

    fn finish_running_interrupted(&mut self, agent_id: &str, now: u64) -> bool {
        let slot = self.runs.iter().position(|run| {
            run.agent_id.as_deref() == Some(agent_id) && run.status == SubagentStatus::Running
        });
        let Some(index) = slot else {
            return false;
        };
        let run = &mut self.runs[index];
        run.status = SubagentStatus::Done;
        run.ended_at_ms = Some(now);
        run.interrupted = true;
        true
    }

    /// `Some(summary)` quando o orquestrador já fechou o turno e não há mais
    /// subagente ativo — a sessão pode descer para `Idle`. `None` mantém como
    /// está.
    fn idle_transition(&self) -> Option<Option<String>> {
        (self.orchestrator_idle && self.active_count() == 0)
            .then(|| self.orchestrator_idle_summary.clone())
    }

    fn end_all(&mut self, now: u64) {
        self.runs
            .retain(|run| run.status != SubagentStatus::Starting);
        for run in &mut self.runs {
            if run.status == SubagentStatus::Running {
                run.status = SubagentStatus::Done;
                run.ended_at_ms = Some(now);
            }
        }
    }

    fn set_focus(&mut self, agent_id: String) {
        if self
            .runs
            .iter()
            .any(|run| run.agent_id.as_deref() == Some(agent_id.as_str()))
        {
            self.focused = Some(agent_id);
        }
    }

    fn to_snapshot(&self) -> SubagentSnapshot {
        // Quem roda vem antes de quem terminou; dentro de cada grupo, o mais
        // novo primeiro. É a ordem da árvore: a rodada viva no topo.
        let mut ordered: Vec<&SubagentRun> = self.runs.iter().collect();
        ordered.sort_by_key(|run| {
            (
                run.status == SubagentStatus::Done,
                std::cmp::Reverse(run.started_at_ms),
            )
        });
        SubagentSnapshot {
            focused: self.focused.clone(),
            subagents: ordered.into_iter().map(SubagentRunDto::redacted).collect(),
        }
    }

    fn running_targets(&self) -> Vec<WatchSpec> {
        self.runs
            .iter()
            .filter(|run| run.status == SubagentStatus::Running)
            .filter_map(|run| {
                Some(WatchSpec {
                    agent_id: run.agent_id.clone()?,
                    agent_type: run.agent_type.clone(),
                    description: (!run.description.is_empty()).then(|| run.description.clone()),
                    parent_path: run.parent_transcript_path.clone(),
                    path: run.transcript_path.clone(),
                    stale_end_len: run.stale_end_len,
                })
            })
            .collect()
    }
}

#[derive(Default)]
pub struct SubagentTracker {
    sessions: SharedSessions,
    watchers: TranscriptWatchers,
    coordinator: SharedCoordinator,
}

impl SubagentTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_idle_coordinator(&self, coordinator: IdleCoordinator) {
        *self.coordinator.lock().expect("subagents coordinator lock") = Some(coordinator);
    }

    pub fn register_session(&self, session: SessionId) {
        self.sessions
            .lock()
            .expect("subagents lock")
            .entry(session)
            .or_default();
    }

    pub fn note_orchestrator_working(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.orchestrator_idle = false;
            entry.orchestrator_idle_summary = None;
        }
    }

    pub fn note_orchestrator_idle(&self, session: SessionId, summary: Option<String>) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.orchestrator_idle = true;
            entry.orchestrator_idle_summary = summary;
        }
    }

    /// O orquestrador encerrou o turno (`Stop`, interrupção). Anota o turno
    /// ocioso, descarta pedidos de spawn que não viraram subagente e devolve
    /// `true` quando ainda há subagente trabalhando — a sessão NÃO está ociosa
    /// (decisão 4 revista). O chamador aplica o status.
    pub fn on_orchestrator_stop<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        summary: Option<String>,
    ) -> bool {
        let (held, payload) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return false;
            };
            entry.orchestrator_idle = true;
            entry.orchestrator_idle_summary = summary;
            let dropped = entry.drop_unconfirmed() > 0;
            (
                entry.active_count() > 0,
                dropped.then(|| changed_payload(session, entry)),
            )
        };
        if let Some(payload) = payload {
            let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        }
        held
    }

    /// Fim de subagente lido do disco (notificação na fila do pai). Só fecha
    /// quem está `Running` — idempotente com o `SubagentStop` e com o fim pelo
    /// sidecar, e nunca cria entrada para id desconhecido. Se era o último e o
    /// orquestrador já tinha encerrado o turno, a sessão desce a `Idle` pelo
    /// coordenador.
    pub fn on_subagent_completed<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_id: &str,
        summary: Option<String>,
        end: RunEnd,
    ) {
        let agent_id = strip_agent_prefix(agent_id);
        if !complete_running(
            app,
            &self.sessions,
            &self.coordinator,
            session,
            &agent_id,
            summary,
            end,
        ) {
            return;
        }
        let targets = self
            .sessions
            .lock()
            .expect("subagents lock")
            .get(&session)
            .map(SessionSubagents::running_targets)
            .unwrap_or_default();
        self.sync_watchers(app, session, targets);
    }

    /// O orquestrador encerrou o turno (e não voltou a trabalhar desde então).
    pub fn orchestrator_idle(&self, session: SessionId) -> bool {
        session_orchestrator_idle(&self.sessions, session)
    }

    pub fn has_active(&self, session: SessionId) -> bool {
        self.sessions
            .lock()
            .expect("subagents lock")
            .get(&session)
            .map(|entry| entry.active_count() > 0)
            .unwrap_or(false)
    }

    pub fn on_spawn_requested<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_type: Option<String>,
        description: Option<String>,
    ) {
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.push_pending(agent_type, description, now_ms());
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn on_spawn_denied<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_type: Option<String>,
        description: Option<String>,
    ) {
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.remove_pending(agent_type, description);
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn on_subagent_started<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_id: String,
        agent_type: String,
        description: Option<String>,
        parent_transcript_path: Option<&Path>,
    ) -> Option<Coordination> {
        // Agente interno do Claude (título, sugestão de prompt…) chega sem
        // `agent_type`: não é subagente que o usuário pediu, e virava a
        // entrada fantasma "subagente" do painel.
        if agent_type.trim().is_empty() {
            return None;
        }
        let agent_id = strip_agent_prefix(&agent_id);
        let hint = {
            let sessions = self.sessions.lock().expect("subagents lock");
            sessions
                .get(&session)
                .and_then(|entry| entry.pending_hint(&agent_type))
                .or_else(|| description.clone())
        };
        let transcript_path = resolve_transcript(
            parent_transcript_path,
            &agent_id,
            &agent_type,
            hint.as_deref(),
        );
        // A descrição de verdade mora no `agent-<id>.meta.json` do próprio
        // subagente. Com vários do mesmo tipo disparados juntos, os starts
        // chegam fora da ordem das chamadas, e casar pelo tipo trocava os nomes.
        let real = transcript_path
            .as_deref()
            .and_then(meta_description)
            .or_else(|| parent_transcript_path.and_then(|p| meta_description_by_id(p, &agent_id)));
        let confirmed = real.is_some();
        let description = real.or(description);
        let (payload, targets, coordination) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let entry = sessions.get_mut(&session)?;
            entry.promote(
                agent_id.clone(),
                agent_type,
                description,
                transcript_path,
                parent_transcript_path,
                now_ms(),
            );
            if confirmed {
                entry.confirm_description(&agent_id);
            }
            let coordination = (!entry.coordinated).then_some(Coordination {
                viewer_disarmed: entry.viewer_disarmed,
                panel_disarmed: entry.panel_disarmed,
            });
            (
                changed_payload(session, entry),
                entry.running_targets(),
                coordination,
            )
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        self.sync_watchers(app, session, targets);
        coordination
    }

    pub fn mark_coordinated(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        sessions.entry(session).or_default().coordinated = true;
    }

    pub fn disarm_viewer(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.viewer_disarmed = true;
        }
    }

    pub fn disarm_panel(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.panel_disarmed = true;
        }
    }

    /// Devolve `Some(summary)` quando o stop deste subagente deixou a sessão
    /// pronta para descer a `Idle` (orquestrador já ocioso, zero subagente
    /// ativo). O chamador (que possui o `SessionManager`) aplica o status.
    pub fn on_subagent_stopped<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_id: String,
        agent_type: Option<String>,
        last_assistant_message: Option<String>,
    ) -> Option<Option<String>> {
        let agent_id = strip_agent_prefix(&agent_id);
        let (payload, targets, idle) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let entry = sessions.get_mut(&session)?;
            if !entry.mark_stopped(agent_id, agent_type, last_assistant_message, now_ms()) {
                return None;
            }
            let idle = entry.idle_transition();
            (
                changed_payload(session, entry),
                entry.running_targets(),
                idle,
            )
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        self.sync_watchers(app, session, targets);
        idle
    }

    /// Um subagente chamou uma ferramenta (`PreToolUse` com `agent_id`): o
    /// painel passa a dizer o que ele está fazendo agora.
    pub fn on_tool_use<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_id: &str,
        activity: ToolActivity,
    ) {
        let agent_id = strip_agent_prefix(agent_id);
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            if !entry.note_activity(&agent_id, activity) {
                return;
            }
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn on_session_ended<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId) {
        let (payload, targets) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.end_all(now_ms());
            (changed_payload(session, entry), entry.running_targets())
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        self.sync_watchers(app, session, targets);
    }

    fn sync_watchers<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        targets: Vec<WatchSpec>,
    ) {
        self.watchers.sync(
            app,
            session,
            targets,
            Arc::clone(&self.sessions),
            Arc::clone(&self.coordinator),
        );
    }

    pub fn focus<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId, agent_id: String) {
        let agent_id = strip_agent_prefix(&agent_id);
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.set_focus(agent_id);
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn remove_session<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId) {
        self.watchers.stop_session(app, session);
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(mut entry) = sessions.remove(&session) else {
                return;
            };
            entry.end_all(now_ms());
            changed_payload(session, &entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn resolve_transcript_path(&self, session: SessionId, agent_id: &str) -> Option<PathBuf> {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        let run = sessions
            .get_mut(&session)?
            .runs
            .iter_mut()
            .find(|run| run.agent_id.as_deref() == Some(agent_id))?;
        if run.transcript_path.is_none() {
            let hint = (!run.description.is_empty()).then_some(run.description.as_str());
            run.transcript_path = resolve_transcript(
                run.parent_transcript_path.as_deref(),
                agent_id,
                &run.agent_type,
                hint,
            );
        }
        run.transcript_path.clone()
    }

    pub fn snapshot(&self, session: SessionId) -> SubagentSnapshot {
        self.sessions
            .lock()
            .expect("subagents lock")
            .get(&session)
            .map(SessionSubagents::to_snapshot)
            .unwrap_or_else(|| SubagentSnapshot {
                focused: None,
                subagents: Vec::new(),
            })
    }
}

fn changed_payload(session: SessionId, entry: &SessionSubagents) -> SubagentsChangedPayload {
    let snapshot = entry.to_snapshot();
    SubagentsChangedPayload {
        session_id: session,
        focused: snapshot.focused,
        subagents: snapshot.subagents,
    }
}

/// Chamado pelo watcher quando um subagente async termina por arquivo. Marca o
/// run `Done` e devolve o snapshot para reemitir e a transição de status
/// pendente (se houver). Roda sob o lock de `sessions`, que é solto antes de
/// tocar em status/app — sem lock aninhado.
pub(crate) fn finish_by_file(
    sessions: &SharedSessions,
    session: SessionId,
    agent_id: &str,
    summary: Option<String>,
    now: u64,
) -> Option<(SubagentSnapshot, Option<Option<String>>)> {
    let mut map = sessions.lock().ok()?;
    let entry = map.get_mut(&session)?;
    if !entry.finish_running_by_file(agent_id, summary, now) {
        return None;
    }
    let idle = entry.idle_transition();
    Some((entry.to_snapshot(), idle))
}

pub(crate) fn session_orchestrator_idle(sessions: &SharedSessions, session: SessionId) -> bool {
    sessions
        .lock()
        .ok()
        .and_then(|map| map.get(&session).map(|entry| entry.orchestrator_idle))
        .unwrap_or(false)
}

pub(crate) fn finish_by_file_blocked(
    sessions: &SharedSessions,
    session: SessionId,
    agent_id: &str,
    summary: Option<String>,
    now: u64,
) -> Option<(SubagentSnapshot, Option<Option<String>>)> {
    let mut map = sessions.lock().ok()?;
    let entry = map.get_mut(&session)?;
    if !entry.finish_running_blocked(agent_id, summary, now) {
        return None;
    }
    // Quando só a marca de "parado" chegou tarde (o run já estava `Done`), a
    // transição pode sair `Some` de novo e o coordenador ser chamado outra
    // vez. Inofensivo: ele só desce sessão `Running`, e a primeira chamada
    // (de quem fechou o run) já a desceu.
    let idle = entry.idle_transition();
    Some((entry.to_snapshot(), idle))
}

pub(crate) fn finish_by_file_interrupted(
    sessions: &SharedSessions,
    session: SessionId,
    agent_id: &str,
    now: u64,
) -> Option<(SubagentSnapshot, Option<Option<String>>)> {
    let mut map = sessions.lock().ok()?;
    let entry = map.get_mut(&session)?;
    if !entry.finish_running_interrupted(agent_id, now) {
        return None;
    }
    let idle = entry.idle_transition();
    Some((entry.to_snapshot(), idle))
}

/// Fecha um subagente `Running` por sinal do disco (sidecar ou fila do pai),
/// reemite o painel e, se a sessão estava segura só por ele, chama o
/// coordenador de `Idle`. `false` quando não havia o que fechar.
pub(crate) fn complete_running<R: Runtime>(
    app: &AppHandle<R>,
    sessions: &SharedSessions,
    coordinator: &SharedCoordinator,
    session: SessionId,
    agent_id: &str,
    summary: Option<String>,
    end: RunEnd,
) -> bool {
    let outcome = match end {
        RunEnd::Interrupted => finish_by_file_interrupted(sessions, session, agent_id, now_ms()),
        RunEnd::Completed => finish_by_file(sessions, session, agent_id, summary, now_ms()),
        RunEnd::Blocked => finish_by_file_blocked(sessions, session, agent_id, summary, now_ms()),
    };
    let Some((snapshot, idle)) = outcome else {
        return false;
    };
    emit_snapshot(app, session, snapshot);
    if let Some(summary) = idle {
        let coordinator = coordinator
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned());
        if let Some(coordinator) = coordinator {
            coordinator(session, summary);
        }
    }
    true
}

pub(crate) fn emit_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    session: SessionId,
    snapshot: SubagentSnapshot,
) {
    let _ = app.emit(
        EVENT_SUBAGENTS_CHANGED,
        SubagentsChangedPayload {
            session_id: session,
            focused: snapshot.focused,
            subagents: snapshot.subagents,
        },
    );
}

pub(crate) fn strip_agent_prefix(agent_id: &str) -> String {
    agent_id
        .strip_prefix("agent-")
        .unwrap_or(agent_id)
        .to_string()
}

pub(crate) fn sidecar_dir(parent_transcript_path: &Path) -> PathBuf {
    parent_transcript_path.with_extension("").join("subagents")
}

fn transcript_by_agent_id(sidecar: &Path, agent_id: &str) -> Option<PathBuf> {
    let bare = agent_id.strip_prefix("agent-").unwrap_or(agent_id);
    let candidate = sidecar.join(format!("agent-{bare}.jsonl"));
    candidate.is_file().then_some(candidate)
}

/// A descrição gravada pelo Claude no `agent-<id>.meta.json` ao lado do
/// transcript do subagente. Leitura curta e defensiva: falha vira `None`.
fn meta_description(transcript: &Path) -> Option<String> {
    let name = transcript.file_name()?.to_str()?;
    let base = name.strip_suffix(".jsonl")?;
    read_meta_description(&transcript.with_file_name(format!("{base}.meta.json")))
}

/// A mesma descrição achada só pelo id, sem depender do transcript existir:
/// `<sessão>/subagents/agent-<id>.meta.json` ao lado do transcript pai.
fn meta_description_by_id(parent_transcript: &Path, agent_id: &str) -> Option<String> {
    let bare = agent_id.strip_prefix("agent-").unwrap_or(agent_id);
    if !is_plausible_subagent_id(bare) {
        return None;
    }
    read_meta_description(&sidecar_dir(parent_transcript).join(format!("agent-{bare}.meta.json")))
}

fn read_meta_description(meta: &Path) -> Option<String> {
    let body = std::fs::read(meta).ok().filter(|b| b.len() <= 64 * 1024)?;
    let value: Value = serde_json::from_slice(&body).ok()?;
    value
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_owned)
}

fn meta_sibling_jsonl(meta_path: &Path) -> PathBuf {
    let name = meta_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let base = name.strip_suffix(".meta.json").unwrap_or(name);
    meta_path.with_file_name(format!("{base}.jsonl"))
}

struct MetaMatch {
    modified: std::time::SystemTime,
    jsonl: PathBuf,
    description_match: bool,
}

fn transcript_by_meta_scan(
    sidecar: &Path,
    agent_type: &str,
    description: Option<&str>,
) -> Option<PathBuf> {
    let mut candidates: Vec<MetaMatch> = Vec::new();
    for entry in std::fs::read_dir(sidecar).ok()?.flatten() {
        let meta_path = entry.path();
        let is_meta = meta_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".meta.json"));
        if !is_meta {
            continue;
        }
        let Some(meta) = std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        else {
            continue;
        };
        if meta.get("agentType").and_then(Value::as_str) != Some(agent_type) {
            continue;
        }
        let jsonl = meta_sibling_jsonl(&meta_path);
        if !jsonl.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let description_match =
            description.is_some() && meta.get("description").and_then(Value::as_str) == description;
        candidates.push(MetaMatch {
            modified,
            jsonl,
            description_match,
        });
    }
    let refined: Vec<&MetaMatch> = candidates.iter().filter(|m| m.description_match).collect();
    let pool: Vec<&MetaMatch> = if refined.is_empty() {
        candidates.iter().collect()
    } else {
        refined
    };
    pool.into_iter()
        .max_by_key(|m| m.modified)
        .map(|m| m.jsonl.clone())
}

pub(crate) fn resolve_transcript(
    parent_transcript_path: Option<&Path>,
    agent_id: &str,
    agent_type: &str,
    description: Option<&str>,
) -> Option<PathBuf> {
    let sidecar = sidecar_dir(parent_transcript_path?);
    transcript_by_agent_id(&sidecar, agent_id)
        .or_else(|| transcript_by_meta_scan(&sidecar, agent_type, description))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn status_of(session: &SessionSubagents, agent_id: &str) -> Option<SubagentStatus> {
        session
            .runs
            .iter()
            .find(|run| run.agent_id.as_deref() == Some(agent_id))
            .map(|run| run.status)
    }

    #[test]
    fn spawn_request_creates_a_starting_run() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("revisar".into()), 10);
        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].status, SubagentStatus::Starting);
        assert!(session.runs[0].agent_id.is_none());
        assert_eq!(session.runs[0].agent_type, "reviewer");
        assert!(session.focused.is_none());
    }

    #[test]
    fn start_promotes_oldest_compatible_pending_among_many() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);
        session.push_pending(Some("reviewer".into()), Some("segundo".into()), 20);
        session.push_pending(Some("explorer".into()), Some("outro tipo".into()), 30);

        session.promote("a1".into(), "reviewer".into(), None, None, None, 40);

        let promoted = session
            .runs
            .iter()
            .find(|run| run.agent_id.as_deref() == Some("a1"))
            .unwrap();
        assert_eq!(promoted.description, "primeiro");
        assert_eq!(promoted.status, SubagentStatus::Running);
        let still_pending = session
            .runs
            .iter()
            .filter(|run| run.status == SubagentStatus::Starting)
            .count();
        assert_eq!(still_pending, 2);
    }

    #[test]
    fn start_without_matching_pending_creates_running_run() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), None, 10);
        session.promote("a9".into(), "explorer".into(), None, None, None, 20);
        assert_eq!(status_of(&session, "a9"), Some(SubagentStatus::Running));
        assert_eq!(
            session
                .runs
                .iter()
                .filter(|run| run.status == SubagentStatus::Starting)
                .count(),
            1
        );
    }

    #[test]
    fn first_running_becomes_default_focus() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        assert_eq!(session.focused.as_deref(), Some("a1"));
        session.promote("a2".into(), "explorer".into(), None, None, None, 20);
        assert_eq!(session.focused.as_deref(), Some("a1"));
    }

    #[test]
    fn focus_stays_on_agent_after_it_finishes() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.promote("a2".into(), "explorer".into(), None, None, None, 20);
        session.set_focus("a2".into());
        session.mark_stopped("a2".into(), None, Some("pronto".into()), 30);
        assert_eq!(session.focused.as_deref(), Some("a2"));
        assert_eq!(status_of(&session, "a2"), Some(SubagentStatus::Done));
    }

    #[test]
    fn focus_ignores_unknown_agent() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.set_focus("ghost".into());
        assert_eq!(session.focused.as_deref(), Some("a1"));
    }

    #[test]
    fn stop_sets_summary_from_last_assistant_message() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.mark_stopped("a1".into(), None, Some("  achei   dois bugs ".into()), 20);
        let run = &session.runs[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.ended_at_ms, Some(20));
        assert_eq!(run.summary.as_deref(), Some("achei dois bugs"));
    }

    #[test]
    fn activity_shows_while_running_and_gives_way_to_the_summary() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        let input = serde_json::json!({ "command": "bun test src/api\nmais" });
        let activity = tool_activity("Bash", Some(&input), "bun test src/api\nmais");
        assert!(session.note_activity("a1", activity.clone()));
        assert!(
            !session.note_activity("a1", activity),
            "mesma atividade não reemite"
        );
        let dto = SubagentRunDto::redacted(&session.runs[0]);
        assert_eq!(dto.activity_tool.as_deref(), Some("Bash"));
        assert_eq!(dto.activity_detail.as_deref(), Some("bun test src/api"));

        session.mark_stopped("a1".into(), None, Some("pronto".into()), 20);
        let dto = SubagentRunDto::redacted(&session.runs[0]);
        assert_eq!(dto.activity_tool, None);
        assert_eq!(dto.activity_detail, None);
        assert_eq!(dto.summary.as_deref(), Some("pronto"));
    }

    #[test]
    fn three_of_the_same_type_keep_their_own_names_when_starts_arrive_out_of_order() {
        // Print do dono (2026-10-14): 3 Explore disparados juntos; os starts
        // chegaram fora da ordem e os nomes trocaram de dono.
        let mut session = SessionSubagents::default();
        for d in [
            "Listar arquivos .rs",
            "Listar arquivos .ts",
            "Contar linhas do README",
        ] {
            session.push_pending(Some("Explore".into()), Some(d.into()), 10);
        }
        // starts: README, .rs, .ts — com a descrição real lida do meta.json
        session.promote(
            "c".into(),
            "Explore".into(),
            Some("Contar linhas do README".into()),
            None,
            None,
            11,
        );
        session.promote(
            "a".into(),
            "Explore".into(),
            Some("Listar arquivos .rs".into()),
            None,
            None,
            12,
        );
        session.promote(
            "b".into(),
            "Explore".into(),
            Some("Listar arquivos .ts".into()),
            None,
            None,
            13,
        );
        let name = |id: &str| {
            session
                .runs
                .iter()
                .find(|r| r.agent_id.as_deref() == Some(id))
                .unwrap()
                .description
                .clone()
        };
        assert_eq!(name("a"), "Listar arquivos .rs");
        assert_eq!(name("b"), "Listar arquivos .ts");
        assert_eq!(name("c"), "Contar linhas do README");
        assert_eq!(
            session.runs.len(),
            3,
            "nenhum placeholder sobrando nem duplicado"
        );
    }

    #[test]
    fn name_guessed_by_order_is_fixed_when_the_meta_json_shows_up() {
        // Print do dono (2026-10-14): 4 general-purpose; no start o meta.json
        // ainda não existia, o palpite por ordem trocou os nomes.
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = sidecar_dir(&parent);
        fs::create_dir_all(&sidecar).unwrap();
        let (id_ls, id_curl) = ("a1b2c3d4e5f6a7b8c", "0f1e2d3c4b5a69788");
        let mut session = SessionSubagents::default();
        session.push_pending(
            Some("general-purpose".into()),
            Some("Run ls -la".into()),
            10,
        );
        session.push_pending(Some("general-purpose".into()), Some("Run curl".into()), 10);
        // o curl começa primeiro e, sem meta.json, leva o nome do primeiro da fila
        session.promote(
            id_curl.into(),
            "general-purpose".into(),
            None,
            None,
            Some(&parent),
            11,
        );
        session.promote(
            id_ls.into(),
            "general-purpose".into(),
            None,
            None,
            Some(&parent),
            12,
        );
        let name = |s: &SessionSubagents, id: &str| {
            s.runs
                .iter()
                .find(|r| r.agent_id.as_deref() == Some(id))
                .unwrap()
                .description
                .clone()
        };
        assert_eq!(
            name(&session, id_curl),
            "Run ls -la",
            "palpite errado de propósito"
        );
        write(
            &sidecar.join(format!("agent-{id_curl}.meta.json")),
            r#"{"description":"Run curl"}"#,
        );
        write(
            &sidecar.join(format!("agent-{id_ls}.meta.json")),
            r#"{"description":"Run ls -la"}"#,
        );
        let input = serde_json::json!({ "command": "curl -s x" });
        assert!(session.note_activity(id_curl, tool_activity("Bash", Some(&input), "curl -s x")));
        assert_eq!(name(&session, id_curl), "Run curl");
        session.mark_stopped(id_ls.into(), None, Some("ok".into()), 20);
        assert_eq!(name(&session, id_ls), "Run ls -la");
    }

    #[test]
    fn meta_description_reads_the_agents_own_meta_json() {
        let dir = TempDir::new().unwrap();
        let jsonl = dir.path().join("agent-abc123.jsonl");
        write(&jsonl, "{}");
        write(
            &dir.path().join("agent-abc123.meta.json"),
            r#"{"agentType":"Explore","description":"  Listar arquivos .ts  "}"#,
        );
        assert_eq!(
            meta_description(&jsonl).as_deref(),
            Some("Listar arquivos .ts")
        );
        assert_eq!(meta_description(&dir.path().join("agent-zzz.jsonl")), None);
    }

    #[test]
    fn recent_activity_keeps_the_last_two_and_clears_on_done() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explore".into(), None, None, None, 10);
        for path in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            let input = serde_json::json!({ "file_path": path });
            assert!(session.note_activity("a1", tool_activity("Read", Some(&input), path)));
        }
        let dto = SubagentRunDto::redacted(&session.runs[0]);
        assert_eq!(dto.activity_detail.as_deref(), Some("d.rs"));
        let recent: Vec<_> = dto
            .recent_activity
            .iter()
            .map(|r| r.detail.as_deref())
            .collect();
        assert_eq!(
            recent,
            vec![Some("c.rs"), Some("b.rs")],
            "mais nova primeiro, só 2"
        );
        assert!(dto.activity_at_ms.is_some());

        session.mark_stopped("a1".into(), None, Some("pronto".into()), 20);
        let dto = SubagentRunDto::redacted(&session.runs[0]);
        assert!(dto.recent_activity.is_empty());
        assert_eq!(dto.activity_at_ms, None);
    }

    #[test]
    fn activity_of_unknown_or_finished_agent_is_ignored() {
        let mut session = SessionSubagents::default();
        let activity = tool_activity("Read", None, "Read");
        assert!(!session.note_activity("ghost", activity.clone()));
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.mark_stopped("a1".into(), None, None, 20);
        assert!(!session.note_activity("a1", activity));
    }

    #[test]
    fn tool_activity_picks_the_argument_that_says_what_it_does() {
        let read = serde_json::json!({ "file_path": "/repo/src/main.rs" });
        assert_eq!(
            tool_activity("Read", Some(&read), "Read"),
            ToolActivity {
                tool: "Read".into(),
                detail: Some("/repo/src/main.rs".into())
            }
        );
        let grep = serde_json::json!({ "pattern": "fn main" });
        assert_eq!(
            tool_activity("Grep", Some(&grep), "Grep").detail.as_deref(),
            Some("fn main")
        );
        // Sem argumento conhecido, o nome sozinho — sem repetir o nome no detalhe.
        assert_eq!(tool_activity("TodoWrite", None, "TodoWrite").detail, None);
        // Ferramenta desconhecida cai na descrição normalizada, sem o nome na frente.
        assert_eq!(
            tool_activity("Mcp", None, "Mcp search docs")
                .detail
                .as_deref(),
            Some("search docs")
        );
        let long = "x".repeat(300);
        let input = serde_json::json!({ "command": long });
        let detail = tool_activity("Bash", Some(&input), "").detail.unwrap();
        assert_eq!(detail.chars().count(), ACTIVITY_MAX_CHARS + 1);
        assert!(detail.ends_with('…'));
    }

    #[test]
    fn stop_summary_truncates_at_280_chars() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.mark_stopped("a1".into(), None, Some("x".repeat(400)), 20);
        assert!(session.runs[0].summary.as_ref().unwrap().chars().count() <= 280);
    }

    #[test]
    fn stop_of_unknown_plausible_agent_creates_orphan_done_run() {
        let mut session = SessionSubagents::default();
        let created = session.mark_stopped(
            "a51505d91f9ef581f".into(),
            Some("general-purpose".into()),
            Some("resumo".into()),
            50,
        );
        assert!(created);
        assert_eq!(session.runs.len(), 1);
        let run = &session.runs[0];
        assert_eq!(run.agent_id.as_deref(), Some("a51505d91f9ef581f"));
        assert_eq!(run.agent_type, "general-purpose");
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.summary.as_deref(), Some("resumo"));
        assert!(session.focused.is_none());
    }

    #[test]
    fn stop_without_plausible_id_creates_no_phantom() {
        let mut session = SessionSubagents::default();
        let created = session.mark_stopped("aX".into(), Some("general-purpose".into()), None, 50);
        assert!(!created);
        assert!(session.runs.is_empty());
    }

    #[test]
    fn stop_without_agent_type_creates_no_phantom() {
        let mut session = SessionSubagents::default();
        let created = session.mark_stopped("a51505d91f9ef581f".into(), None, None, 50);
        assert!(!created);
        assert!(session.runs.is_empty());
        let blank = session.mark_stopped(
            "a51505d91f9ef581f".into(),
            Some("   ".into()),
            Some("resumo".into()),
            50,
        );
        assert!(!blank);
        assert!(session.runs.is_empty());
    }

    #[test]
    fn session_end_marks_running_as_done_and_discards_starting() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.push_pending(Some("explorer".into()), None, 20);
        session.end_all(99);
        let running_done = session
            .runs
            .iter()
            .find(|run| run.agent_id.as_deref() == Some("a1"))
            .unwrap();
        assert_eq!(running_done.status, SubagentStatus::Done);
        assert_eq!(running_done.ended_at_ms, Some(99));
        assert!(running_done.summary.is_none());
        assert!(!session
            .runs
            .iter()
            .any(|run| run.status == SubagentStatus::Starting));
    }

    #[test]
    fn deny_removes_newest_compatible_pending_among_many() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);
        session.push_pending(Some("reviewer".into()), Some("segundo".into()), 20);
        session.push_pending(Some("explorer".into()), Some("outro".into()), 30);

        session.remove_pending(Some("reviewer".into()), Some("segundo".into()));

        assert_eq!(session.runs.len(), 2);
        assert!(session.runs.iter().any(|run| run.description == "primeiro"));
        assert!(session.runs.iter().any(|run| run.description == "outro"));
        assert!(!session.runs.iter().any(|run| run.description == "segundo"));
    }

    #[test]
    fn deny_without_description_removes_newest_of_type() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);
        session.push_pending(Some("reviewer".into()), Some("segundo".into()), 20);

        session.remove_pending(Some("reviewer".into()), None);

        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].description, "primeiro");
    }

    #[test]
    fn deny_without_matching_pending_is_noop() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);

        session.remove_pending(Some("explorer".into()), None);

        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].status, SubagentStatus::Starting);
    }

    #[test]
    fn duplicate_stop_does_not_change_runs() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        assert!(session.mark_stopped("a1".into(), None, Some("pronto".into()), 20));
        let len_after_first = session.runs.len();
        assert!(!session.mark_stopped("a1".into(), None, Some("de novo".into()), 30));
        assert_eq!(session.runs.len(), len_after_first);
        assert_eq!(session.runs[0].summary.as_deref(), Some("pronto"));
        assert_eq!(session.runs[0].ended_at_ms, Some(20));
    }

    #[test]
    fn snapshot_orders_newest_first_and_redacts() {
        let mut session = SessionSubagents::default();
        session.push_pending(
            Some("reviewer".into()),
            Some("token sk-abcdef1234567890ABCDEFghijkl aqui".into()),
            30,
        );
        session.push_pending(Some("explorer".into()), Some("primeiro".into()), 10);
        let snapshot = session.to_snapshot();
        assert_eq!(snapshot.subagents.len(), 2);
        assert_eq!(snapshot.subagents[0].started_at_ms, 30);
        assert_eq!(snapshot.subagents[1].started_at_ms, 10);
        assert!(!snapshot.subagents[0]
            .description
            .contains("sk-abcdef1234567890ABCDEFghijkl"));
        assert!(snapshot.subagents[0].description.contains("[REDACTED]"));
    }

    /// A5.3, tela 4 do dono: duas rodadas de subagentes na MESMA sessão
    /// ("de novo por favor"). A segunda rodada não pode apagar a primeira, e
    /// quem roda vem no topo, acima de quem terminou. Dados sintéticos.
    #[test]
    fn second_round_lists_both_rounds_running_on_top() {
        let mut session = SessionSubagents::default();
        for (i, id) in ["aaaa0001", "aaaa0002", "aaaa0003", "aaaa0004"]
            .iter()
            .enumerate()
        {
            session.promote(
                (*id).into(),
                "general-purpose".into(),
                Some(format!("Teste {}: rodada 1", i + 1)),
                None,
                None,
                100 + i as u64,
            );
        }
        for id in ["aaaa0001", "aaaa0002", "aaaa0003", "aaaa0004"] {
            assert!(session.mark_stopped(id.into(), None, Some("ok".into()), 200));
        }

        for (i, id) in ["bbbb0001", "bbbb0002", "bbbb0003", "bbbb0004"]
            .iter()
            .enumerate()
        {
            session.push_pending(
                Some("general-purpose".into()),
                Some(format!("Teste {}: rodada 2", i + 1)),
                300 + i as u64,
            );
            session.promote(
                (*id).into(),
                "general-purpose".into(),
                None,
                None,
                None,
                300 + i as u64,
            );
        }
        // Dois da rodada 2 já terminaram; dois seguem rodando.
        session.mark_stopped("bbbb0001".into(), None, None, 320);
        session.mark_stopped("bbbb0002".into(), None, None, 321);

        let snapshot = session.to_snapshot();
        let ids: Vec<&str> = snapshot
            .subagents
            .iter()
            .map(|run| run.agent_id.as_deref().unwrap())
            .collect();
        assert_eq!(snapshot.subagents.len(), 8, "as duas rodadas aparecem");
        assert_eq!(
            ids,
            [
                "bbbb0004", "bbbb0003", // rodando, mais novo primeiro
                "bbbb0002", "bbbb0001", // rodada 2 concluída
                "aaaa0004", "aaaa0003", "aaaa0002", "aaaa0001", // rodada 1
            ]
        );
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Running);
        assert_eq!(snapshot.subagents[2].status, SubagentStatus::Done);
        assert_eq!(snapshot.focused.as_deref(), Some("bbbb0001"));
        let rounds: Vec<u32> = snapshot.subagents.iter().map(|run| run.round).collect();
        assert_eq!(
            rounds,
            [2, 2, 2, 2, 1, 1, 1, 1],
            "cada nó sabe a sua rodada"
        );
    }

    #[test]
    fn settled_history_is_capped_without_touching_active() {
        let mut session = SessionSubagents::default();
        for round in 0..(MAX_SETTLED_RUNS as u64 + 5) {
            let id = format!("cafe{round:08x}");
            session.promote(id.clone(), "explorer".into(), None, None, None, round);
            session.mark_stopped(id, None, None, round);
        }
        session.promote(
            "beef00000001".into(),
            "explorer".into(),
            None,
            None,
            None,
            999,
        );
        let snapshot = session.to_snapshot();
        assert_eq!(
            snapshot.subagents[0].agent_id.as_deref(),
            Some("beef00000001")
        );
        assert!(snapshot.subagents.len() <= MAX_SETTLED_RUNS + 1);
        // O mais antigo saiu, o mais novo dos concluídos ficou.
        assert!(!snapshot
            .subagents
            .iter()
            .any(|run| run.agent_id.as_deref() == Some("cafe00000000")));
    }

    #[test]
    fn sidecar_dir_strips_jsonl_and_joins_subagents() {
        let parent = Path::new("/home/x/.claude/projects/slug/session-42.jsonl");
        assert_eq!(
            sidecar_dir(parent),
            Path::new("/home/x/.claude/projects/slug/session-42/subagents")
        );
    }

    fn write(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    #[test]
    fn resolve_finds_file_by_agent_id_with_and_without_prefix() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        let jsonl = sidecar.join("agent-a17hex.jsonl");
        write(&jsonl, "{}");

        assert_eq!(
            resolve_transcript(Some(&parent), "a17hex", "reviewer", None),
            Some(jsonl.clone())
        );
        assert_eq!(
            resolve_transcript(Some(&parent), "agent-a17hex", "reviewer", None),
            Some(jsonl)
        );
    }

    #[test]
    fn resolve_falls_back_to_meta_scan_matching_agent_type() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();

        write(
            &sidecar.join("agent-other.meta.json"),
            r#"{"agentType":"explorer","description":"outro"}"#,
        );
        write(&sidecar.join("agent-other.jsonl"), "{}");
        write(
            &sidecar.join("agent-match.meta.json"),
            r#"{"agentType":"reviewer","description":"revisar diff"}"#,
        );
        let wanted = sidecar.join("agent-match.jsonl");
        write(&wanted, "{}");

        let resolved = resolve_transcript(
            Some(&parent),
            "hook-id-not-a-file",
            "reviewer",
            Some("revisar diff"),
        );
        assert_eq!(resolved, Some(wanted));
    }

    #[test]
    fn resolve_returns_none_when_sidecar_missing() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        assert_eq!(
            resolve_transcript(Some(&parent), "aX", "reviewer", None),
            None
        );
    }

    #[test]
    fn resolve_returns_none_without_parent_path() {
        assert_eq!(resolve_transcript(None, "aX", "reviewer", None), None);
    }

    #[test]
    fn promote_resolves_transcript_path_for_running_run() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        let jsonl = sidecar.join("agent-a1.jsonl");
        write(&jsonl, "{}");

        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("revisar".into()), 10);
        let hint = session.pending_hint("reviewer");
        let path = resolve_transcript(Some(&parent), "a1", "reviewer", hint.as_deref());
        session.promote("a1".into(), "reviewer".into(), None, path, None, 20);
        assert_eq!(session.runs[0].transcript_path, Some(jsonl));
    }

    #[test]
    fn tracker_emits_and_snapshots_across_transitions() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_spawn_requested(
            &handle,
            session,
            Some("reviewer".into()),
            Some("revisar".into()),
        );
        tracker.on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None);
        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.focused.as_deref(), Some("a1"));
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Running);

        tracker.on_session_ended(&handle, session);
        assert_eq!(
            tracker.snapshot(session).subagents[0].status,
            SubagentStatus::Done
        );

        tracker.remove_session(&handle, session);
        assert!(tracker.snapshot(session).subagents.is_empty());
    }

    /// Subagente do Codex: `agent_id` é id de thread (com hífens, não hex), não
    /// há sidecar `subagents/agent-<id>.jsonl` ao lado do rollout, e quem
    /// fecha o nó é o `SubagentStop` com `last_assistant_message`.
    #[test]
    fn codex_subagent_runs_through_start_and_stop_hooks() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        let rollout = dir.path().join("rollout-2026-10-12T10-00-00-thr.jsonl");
        fs::write(&rollout, "").unwrap();
        let agent_id = "019a3f2e-7c1b-7d40-9f3e-5b2c8a1d4e6f";

        tracker.register_session(session);
        tracker.on_subagent_started(
            &handle,
            session,
            agent_id.into(),
            "worker".into(),
            None,
            Some(&rollout),
        );
        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 1);
        let run = &snapshot.subagents[0];
        assert_eq!(run.agent_id.as_deref(), Some(agent_id));
        assert_eq!(run.agent_type, "worker");
        assert_eq!(run.status, SubagentStatus::Running);
        assert_eq!(snapshot.focused.as_deref(), Some(agent_id));
        assert_eq!(tracker.resolve_transcript_path(session, agent_id), None);

        tracker.note_orchestrator_idle(session, Some("orquestrador pronto".into()));
        let idle = tracker.on_subagent_stopped(
            &handle,
            session,
            agent_id.into(),
            Some("worker".into()),
            Some("Testes verdes.".into()),
        );
        assert_eq!(idle, Some(Some("orquestrador pronto".into())));
        let run = &tracker.snapshot(session).subagents[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.summary.as_deref(), Some("Testes verdes."));
        assert!(!tracker.has_active(session));
    }

    #[test]
    fn events_after_remove_session_are_noop() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None);
        tracker.remove_session(&handle, session);

        let restarted = tracker.on_subagent_started(
            &handle,
            session,
            "a2".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(restarted.is_none());
        tracker.on_subagent_stopped(&handle, session, "a2".into(), None, Some("pronto".into()));
        tracker.on_spawn_requested(&handle, session, Some("explorer".into()), None);

        assert!(tracker.snapshot(session).subagents.is_empty());
        assert!(!tracker
            .sessions
            .lock()
            .expect("subagents lock")
            .contains_key(&session));
    }

    #[test]
    fn start_and_stop_correlate_across_agent_prefix() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(
            &handle,
            session,
            "agent-abc".into(),
            "reviewer".into(),
            None,
            None,
        );
        tracker.on_subagent_stopped(&handle, session, "abc".into(), None, Some("pronto".into()));

        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
        assert_eq!(snapshot.subagents[0].agent_id.as_deref(), Some("abc"));
        assert_eq!(snapshot.subagents[0].summary.as_deref(), Some("pronto"));
    }

    #[test]
    fn coordination_retries_until_marked() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        let first = tracker.on_subagent_started(
            &handle,
            session,
            "a1".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(first.is_some());

        let retry = tracker.on_subagent_started(
            &handle,
            session,
            "a2".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(retry.is_some());

        tracker.mark_coordinated(session);

        let after = tracker.on_subagent_started(
            &handle,
            session,
            "a3".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(after.is_none());
    }

    #[test]
    fn new_round_rearms_coordination_and_disarms() {
        // Rodada 1 abre o painel (coordination), conclui, e o usuário pode até
        // ter fechado o painel (disarm). Rodada 2 é contexto novo: coordena de
        // novo — sem isso a segunda rodada roda às cegas.
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        let first = tracker
            .on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None)
            .expect("rodada 1 coordena");
        assert!(!first.panel_disarmed);
        tracker.mark_coordinated(session);
        tracker.disarm_panel(session);
        tracker.on_subagent_stopped(&handle, session, "a1".into(), None, None);

        let second = tracker
            .on_subagent_started(&handle, session, "b2".into(), "reviewer".into(), None, None)
            .expect("rodada 2 re-coordena");
        assert!(!second.panel_disarmed);
        assert!(!second.viewer_disarmed);
        tracker.mark_coordinated(session);

        let same_round = tracker.on_subagent_started(
            &handle,
            session,
            "c3".into(),
            "explorer".into(),
            None,
            None,
        );
        assert!(same_round.is_none(), "coordenação continua once POR RODADA");
    }

    /// "Fechei a janela de subagentes e nunca mais consigo abrir": um spawn
    /// que ficou `Starting` (o `SubagentStart` dele nunca chegou) segurava a
    /// rodada como "em andamento" para sempre, e o `panel_disarmed` do
    /// fechamento grudava. A rodada seguinte tem que re-coordenar.
    #[test]
    fn closed_panel_rearms_even_with_a_stuck_starting_placeholder() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);

        tracker.on_spawn_requested(&handle, session, Some("explorer".into()), Some("a".into()));
        tracker.on_spawn_requested(&handle, session, Some("ghost".into()), Some("b".into()));
        tracker
            .on_subagent_started(
                &handle,
                session,
                "aaaa1111".into(),
                "explorer".into(),
                None,
                None,
            )
            .expect("rodada 1 coordena");
        tracker.mark_coordinated(session);
        tracker.on_subagent_stopped(&handle, session, "aaaa1111".into(), None, None);
        tracker.disarm_panel(session); // o dono fechou o painel

        let next = tracker
            .on_subagent_started(
                &handle,
                session,
                "bbbb2222".into(),
                "reviewer".into(),
                None,
                None,
            )
            .expect("rodada nova re-coordena mesmo com placeholder preso");
        assert!(!next.panel_disarmed, "fechar não suprime a rodada seguinte");
    }

    #[test]
    fn coordination_carries_disarm_flags() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.disarm_viewer(session);
        let coordination = tracker
            .on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None)
            .unwrap();
        assert!(coordination.viewer_disarmed);
        assert!(!coordination.panel_disarmed);
    }

    #[test]
    fn spawn_denied_removes_pending_and_reemits() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_spawn_requested(
            &handle,
            session,
            Some("reviewer".into()),
            Some("revisar".into()),
        );
        assert_eq!(tracker.snapshot(session).subagents.len(), 1);

        tracker.on_spawn_denied(
            &handle,
            session,
            Some("reviewer".into()),
            Some("revisar".into()),
        );
        assert!(tracker.snapshot(session).subagents.is_empty());
    }

    #[test]
    fn session_end_leaves_no_running_or_starting() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None);
        tracker.on_spawn_requested(
            &handle,
            session,
            Some("explorer".into()),
            Some("mapear".into()),
        );
        tracker.on_session_ended(&handle, session);

        let snapshot = tracker.snapshot(session);
        assert!(snapshot
            .subagents
            .iter()
            .all(|run| run.status == SubagentStatus::Done));
    }

    #[test]
    fn transcript_path_is_re_resolved_when_sidecar_appears_late() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(
            &handle,
            session,
            "a1".into(),
            "reviewer".into(),
            None,
            Some(&parent),
        );
        assert!(tracker.resolve_transcript_path(session, "a1").is_none());

        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        let jsonl = sidecar.join("agent-a1.jsonl");
        write(&jsonl, "{}");

        assert_eq!(tracker.resolve_transcript_path(session, "a1"), Some(jsonl));
        tracker.remove_session(&handle, session);
    }

    #[test]
    fn reopen_records_the_stale_sidecar_length() {
        let dir = TempDir::new().unwrap();
        let jsonl = dir.path().join("agent-a1.jsonl");
        write(&jsonl, "{\"type\":\"user\"}\n");
        let mut session = SessionSubagents::default();
        session.promote(
            "a1".into(),
            "reviewer".into(),
            None,
            Some(jsonl.clone()),
            None,
            10,
        );
        assert_eq!(session.runs[0].stale_end_len, None);
        assert!(session.finish_running_by_file("a1", Some("pronto".into()), 20));
        // Acordado: o fim que já está no arquivo é velho.
        session.promote("a1".into(), "reviewer".into(), None, None, None, 30);
        assert_eq!(session.runs[0].status, SubagentStatus::Running);
        let len = fs::metadata(&jsonl).unwrap().len();
        assert_eq!(session.runs[0].stale_end_len, Some(len));
        assert_eq!(session.running_targets()[0].stale_end_len, Some(len));
    }

    #[test]
    fn active_count_reflects_starting_and_running() {
        let mut session = SessionSubagents::default();
        assert_eq!(session.active_count(), 0);
        session.push_pending(Some("reviewer".into()), None, 10);
        assert_eq!(session.active_count(), 1);
        session.promote("a1".into(), "explorer".into(), None, None, None, 20);
        assert_eq!(session.active_count(), 2);
        session.mark_stopped("a1".into(), None, Some("pronto".into()), 30);
        assert_eq!(session.active_count(), 1);
    }

    #[test]
    fn finish_by_file_marks_running_done_with_summary() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        assert!(session.finish_running_by_file("a1", Some("  fim  do  turno ".into()), 40));
        let run = &session.runs[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.ended_at_ms, Some(40));
        assert_eq!(run.summary.as_deref(), Some("fim do turno"));
    }

    #[test]
    fn finish_by_file_ignores_starting_and_done() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), None, 10);
        assert!(!session.finish_running_by_file("a1", Some("x".into()), 40));
        session.promote("a1".into(), "reviewer".into(), None, None, None, 20);
        assert!(session.finish_running_by_file("a1", None, 40));
        assert!(!session.finish_running_by_file("a1", Some("de novo".into()), 50));
        assert_eq!(session.runs[0].status, SubagentStatus::Done);
        assert_eq!(session.runs[0].ended_at_ms, Some(40));
    }

    #[test]
    fn finish_running_interrupted_marks_running_done_without_summary() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        assert!(session.finish_running_interrupted("a1", 40));
        let run = &session.runs[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.ended_at_ms, Some(40));
        assert!(run.interrupted);
        assert!(run.summary.is_none());
    }

    #[test]
    fn finish_running_interrupted_ignores_starting_and_done() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), None, 10);
        assert!(!session.finish_running_interrupted("a1", 40));
        session.promote("a1".into(), "reviewer".into(), None, None, None, 20);
        assert!(session.finish_running_interrupted("a1", 40));
        assert!(!session.finish_running_interrupted("a1", 50));
        assert_eq!(session.runs[0].ended_at_ms, Some(40));
    }

    #[test]
    fn interrupted_run_is_serialized_in_snapshot() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        session.finish_running_interrupted("a1", 40);
        let snapshot = session.to_snapshot();
        assert!(snapshot.subagents[0].interrupted);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
    }

    #[test]
    fn finish_by_file_interrupted_flips_last_active_to_idle() {
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        {
            let mut sessions = tracker.sessions.lock().unwrap();
            let entry = sessions.get_mut(&session).unwrap();
            entry.promote("a1".into(), "explorer".into(), None, None, None, 10);
            entry.orchestrator_idle = true;
            entry.orchestrator_idle_summary = Some("turno".into());
        }
        assert!(session_orchestrator_idle(&tracker.sessions, session));
        let (snapshot, idle) =
            finish_by_file_interrupted(&tracker.sessions, session, "a1", 99).unwrap();
        assert!(snapshot.subagents[0].interrupted);
        assert_eq!(idle, Some(Some("turno".into())));
    }

    #[test]
    fn session_orchestrator_idle_defaults_false_without_signal() {
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        assert!(!session_orchestrator_idle(&tracker.sessions, session));
    }

    #[test]
    fn idle_transition_only_when_orchestrator_idle_and_no_active() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        assert!(session.idle_transition().is_none());
        session.orchestrator_idle = true;
        session.orchestrator_idle_summary = Some("resumo do turno".into());
        assert!(session.idle_transition().is_none());
        session.finish_running_by_file("a1", None, 20);
        assert_eq!(
            session.idle_transition(),
            Some(Some("resumo do turno".into()))
        );
    }

    #[test]
    fn has_active_true_with_running_subagent() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        assert!(!tracker.has_active(session));
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);
        assert!(tracker.has_active(session));
    }

    #[test]
    fn file_finish_flips_session_to_idle_when_orchestrator_idle() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);

        type Flips = Arc<Mutex<Vec<(SessionId, Option<String>)>>>;
        let flips: Flips = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&flips);
        tracker.set_idle_coordinator(Arc::new(move |s, summary| {
            sink.lock().unwrap().push((s, summary));
        }));

        // Orquestrador ocioso, mas subagente ainda ativo: nada acontece ainda.
        tracker.note_orchestrator_idle(session, Some("turno pronto".into()));

        let (snapshot, idle) =
            finish_by_file(&tracker.sessions, session, "a1", Some("fim".into()), 99).unwrap();
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
        // simula o watcher aplicando a transição via coordinator
        if let Some(summary) = idle {
            if let Some(coordinator) = tracker.coordinator.lock().unwrap().clone() {
                coordinator(session, summary);
            }
        }
        let recorded = flips.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, session);
        assert_eq!(recorded[0].1.as_deref(), Some("turno pronto"));
    }

    #[test]
    fn subagent_stop_returns_idle_transition_when_last_one_ends() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);
        tracker.note_orchestrator_idle(session, Some("turno".into()));

        let idle = tracker.on_subagent_stopped(
            &handle,
            session,
            "a1".into(),
            Some("explorer".into()),
            Some("achei".into()),
        );
        assert_eq!(idle, Some(Some("turno".into())));
    }

    #[test]
    fn subagent_stop_holds_running_while_another_active() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);
        tracker.on_subagent_started(&handle, session, "a2".into(), "reviewer".into(), None, None);
        tracker.note_orchestrator_idle(session, Some("turno".into()));

        let idle = tracker.on_subagent_stopped(
            &handle,
            session,
            "a1".into(),
            Some("explorer".into()),
            Some("achei".into()),
        );
        assert!(idle.is_none());
        assert!(tracker.has_active(session));
    }

    #[test]
    fn working_signal_clears_orchestrator_idle() {
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.note_orchestrator_idle(session, Some("turno".into()));
        tracker.note_orchestrator_working(session);
        let sessions = tracker.sessions.lock().unwrap();
        let entry = sessions.get(&session).unwrap();
        assert!(!entry.orchestrator_idle);
        assert!(entry.orchestrator_idle_summary.is_none());
    }

    #[test]
    fn session_end_freezes_active_subagent_as_done_without_phantom() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        // subagente async ainda aberto no tracker
        tracker.on_subagent_started(
            &handle,
            session,
            "a51505d91f9ef581f".into(),
            "general-purpose".into(),
            None,
            None,
        );
        // item de fila / stop espúrio não vira entrada
        tracker.on_subagent_stopped(
            &handle,
            session,
            "queued".into(),
            None,
            Some("prompt".into()),
        );

        tracker.on_session_ended(&handle, session);

        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
        assert!(snapshot.subagents[0].ended_at_ms.is_some());
        assert!(!snapshot.subagents.iter().any(|run| matches!(
            run.status,
            SubagentStatus::Running | SubagentStatus::Starting
        )));
    }

    /// A sequência de hooks de uma rodada em background no TUI interativo,
    /// como o Claude Code 2.1.300 manda (forma levantada no E2E de
    /// 2026-10-14): `SubagentStop` de agente interno (sem `agent_type`) a cada
    /// turno, `SubagentStop` sem `last_assistant_message` para quem entrega
    /// pelo `SubagentHandback`, e o subagente acordado de novo pela própria
    /// tarefa de shell (`SubagentStart` repetido com o mesmo id).
    #[test]
    fn real_async_hook_sequence_keeps_one_entry_per_subagent() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        const A: &str = "a1111111111111111";
        const B: &str = "a2222222222222222";
        const INTERNAL: &str = "a3333333333333333";
        let gp = || Some("general-purpose".to_string());

        tracker.on_spawn_requested(&handle, session, gp(), Some("tarefa A".into()));
        tracker.on_spawn_requested(&handle, session, gp(), Some("tarefa B".into()));
        tracker.on_subagent_started(
            &handle,
            session,
            A.into(),
            "general-purpose".into(),
            None,
            None,
        );
        tracker.on_subagent_started(
            &handle,
            session,
            B.into(),
            "general-purpose".into(),
            None,
            None,
        );
        // Turno do orquestrador acaba com os dois em background: segura.
        assert!(tracker.on_orchestrator_stop(&handle, session, Some("lancei".into())));
        tracker.on_subagent_stopped(
            &handle,
            session,
            INTERNAL.into(),
            Some(String::new()),
            Some("título".into()),
        );
        tracker.note_orchestrator_working(session);
        tracker.on_subagent_stopped(&handle, session, B.into(), gp(), None);
        tracker.on_subagent_stopped(&handle, session, A.into(), gp(), Some("dois".into()));
        assert!(!tracker.on_orchestrator_stop(&handle, session, Some("ok".into())));
        tracker.on_subagent_stopped(&handle, session, INTERNAL.into(), Some(String::new()), None);
        assert_eq!(tracker.snapshot(session).subagents.len(), 2);

        // A tarefa de shell do A termina e o acorda: mesma entrada, de volta.
        tracker.on_subagent_started(
            &handle,
            session,
            A.into(),
            "general-purpose".into(),
            None,
            None,
        );
        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 2, "acordar não duplica");
        let a = snapshot
            .subagents
            .iter()
            .find(|r| r.agent_id.as_deref() == Some(A))
            .unwrap();
        assert_eq!(a.status, SubagentStatus::Running);
        assert_eq!(a.description, "tarefa A");
        assert!(a.ended_at_ms.is_none());
        tracker.on_subagent_stopped(&handle, session, A.into(), gp(), None);
        assert!(!tracker.has_active(session));
        assert_eq!(tracker.snapshot(session).subagents.len(), 2);
    }

    #[test]
    fn start_without_agent_type_is_not_a_subagent() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        let coordination = tracker.on_subagent_started(
            &handle,
            session,
            "a3333333333333333".into(),
            "  ".into(),
            Some("já terminaram?".into()),
            None,
        );
        assert!(coordination.is_none());
        assert!(tracker.snapshot(session).subagents.is_empty());
        assert!(!tracker.has_active(session));
    }

    #[test]
    fn orchestrator_stop_drops_spawns_that_never_started() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_spawn_requested(&handle, session, None, Some("fantasma".into()));
        assert!(tracker.has_active(session));
        // Nenhum `SubagentStart` veio no turno: o placeholder não era subagente.
        assert!(!tracker.on_orchestrator_stop(&handle, session, None));
        assert!(tracker.snapshot(session).subagents.is_empty());
    }

    #[test]
    fn untyped_spawn_is_confirmed_by_the_typed_start() {
        let mut session = SessionSubagents::default();
        // `Agent` sem `subagent_type`: o Claude escolhe o tipo, o hook diz qual.
        session.push_pending(None, Some("mapear".into()), 10);
        session.promote(
            "a1b2c3d4e5".into(),
            "general-purpose".into(),
            None,
            None,
            None,
            20,
        );
        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].agent_type, "general-purpose");
        assert_eq!(session.runs[0].description, "mapear");
        assert_eq!(session.runs[0].status, SubagentStatus::Running);
    }

    #[test]
    fn disk_and_hook_completions_are_idempotent_whichever_comes_first() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        let flips = Arc::new(Mutex::new(0));
        let sink = Arc::clone(&flips);
        tracker.set_idle_coordinator(Arc::new(move |_, _| *sink.lock().unwrap() += 1));
        for id in ["a1b2c3d4e5f60718", "b1b2c3d4e5f60718"] {
            tracker.on_subagent_started(&handle, session, id.into(), "explorer".into(), None, None);
        }
        tracker.on_orchestrator_stop(&handle, session, Some("turno".into()));

        // Disco primeiro, hook depois.
        tracker.on_subagent_completed(
            &handle,
            session,
            "a1b2c3d4e5f60718",
            Some("do disco".into()),
            RunEnd::Completed,
        );
        assert_eq!(
            tracker.on_subagent_stopped(
                &handle,
                session,
                "a1b2c3d4e5f60718".into(),
                Some("explorer".into()),
                Some("do hook".into())
            ),
            None
        );
        // Hook primeiro, disco depois.
        assert!(tracker
            .on_subagent_stopped(
                &handle,
                session,
                "b1b2c3d4e5f60718".into(),
                Some("explorer".into()),
                Some("fim".into())
            )
            .is_some());
        tracker.on_subagent_completed(
            &handle,
            session,
            "b1b2c3d4e5f60718",
            None,
            RunEnd::Interrupted,
        );
        // Fim de quem o tracker nunca viu não cria nada.
        tracker.on_subagent_completed(
            &handle,
            session,
            "c1b2c3d4e5f60718",
            None,
            RunEnd::Completed,
        );

        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 2);
        let a = snapshot
            .subagents
            .iter()
            .find(|r| r.agent_id.as_deref() == Some("a1b2c3d4e5f60718"))
            .unwrap();
        assert_eq!(a.summary.as_deref(), Some("do disco"));
        let b = snapshot
            .subagents
            .iter()
            .find(|r| r.agent_id.as_deref() == Some("b1b2c3d4e5f60718"))
            .unwrap();
        assert_eq!(b.summary.as_deref(), Some("fim"));
        assert!(!b.interrupted);
        // O hook devolveu a transição ao chamador; o disco nunca a repete.
        assert_eq!(*flips.lock().unwrap(), 0);
    }

    #[test]
    fn late_blocked_after_hook_stop_marks_the_done_run_once() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_subagent_started(
            &handle,
            session,
            "a1b2c3d4e5f60718".into(),
            "explorer".into(),
            None,
            None,
        );
        // O hook fecha primeiro, sem fala final.
        tracker.on_subagent_stopped(
            &handle,
            session,
            "a1b2c3d4e5f60718".into(),
            Some("explorer".into()),
            None,
        );
        let ended_at = tracker.snapshot(session).subagents[0].ended_at_ms;
        // Depois a fila do pai traz o `blocked`.
        tracker.on_subagent_completed(
            &handle,
            session,
            "a1b2c3d4e5f60718",
            Some("preciso da senha do banco".into()),
            RunEnd::Blocked,
        );
        let run = tracker.snapshot(session).subagents[0].clone();
        assert_eq!(run.status, SubagentStatus::Done);
        assert!(run.blocked);
        assert!(!run.interrupted);
        assert_eq!(run.ended_at_ms, ended_at);
        assert_eq!(run.summary.as_deref(), Some("preciso da senha do banco"));

        // Repetir não muda nada.
        tracker.on_subagent_completed(
            &handle,
            session,
            "a1b2c3d4e5f60718",
            Some("outro texto".into()),
            RunEnd::Blocked,
        );
        let again = tracker.snapshot(session).subagents[0].clone();
        assert!(again.blocked);
        assert_eq!(again.summary.as_deref(), Some("preciso da senha do banco"));
        assert_eq!(again.ended_at_ms, ended_at);
    }

    #[test]
    fn late_blocked_keeps_hook_summary_and_skips_interrupted_runs() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        session.promote("b1".into(), "explorer".into(), None, None, None, 10);
        session.finish_running_by_file("a1", Some("fala do hook".into()), 20);
        session.finish_running_interrupted("b1", 20);

        assert!(session.finish_running_blocked("a1", Some("do disco".into()), 30));
        assert!(!session.finish_running_blocked("a1", None, 40));
        assert!(!session.finish_running_blocked("b1", None, 40));

        let a = session
            .runs
            .iter()
            .find(|r| r.agent_id.as_deref() == Some("a1"))
            .unwrap();
        assert!(a.blocked);
        assert_eq!(a.summary.as_deref(), Some("fala do hook"));
        assert_eq!(a.ended_at_ms, Some(20));
        let b = session
            .runs
            .iter()
            .find(|r| r.agent_id.as_deref() == Some("b1"))
            .unwrap();
        assert!(!b.blocked);
    }

    #[test]
    fn disk_completion_of_the_last_subagent_settles_the_held_session() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        let flips: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&flips);
        tracker.set_idle_coordinator(Arc::new(move |_, summary| {
            sink.lock().unwrap().push(summary)
        }));
        tracker.on_subagent_started(
            &handle,
            session,
            "a1b2c3d4e5f60718".into(),
            "explorer".into(),
            None,
            None,
        );
        assert!(tracker.on_orchestrator_stop(&handle, session, Some("turno".into())));
        tracker.on_subagent_completed(
            &handle,
            session,
            "a1b2c3d4e5f60718",
            Some("ok".into()),
            RunEnd::Completed,
        );
        assert_eq!(*flips.lock().unwrap(), vec![Some("turno".to_string())]);
    }
}

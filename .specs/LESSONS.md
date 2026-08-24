# LESSONS — caixa de entrada

<!-- Escrito pela validação, lido por gente. NUNCA carregado como orientação em
     nenhuma fase. Promover para o AGENTS/CLAUDE.md (disciplina de teste e
     código) ou para um AD-NNN no STATE.md (arquitetural) é ato humano; ao
     promover, apagar a linha daqui. -->

## Inbox

- [mutante sobrevivente] Quando uma checagem parece redundante com a conversão que vem depois, cobrir no teste o caso em que as duas discordam — é ele que decide se a checagem fica ou sai · evidence: `src-tauri/src/history/import/parser/bash.rs:96` (`#-123` seria data sem a checagem de dígitos) · features: shell-history-import
- [lacuna de cobertura] Feature que grava dado novo numa tabela compartilhada precisa de teste do caminho de **limpeza** também, não só do de escrita · evidence: `src-tauri/src/session/store.rs:2225` · features: shell-history-import
- [lacuna de cobertura] Idempotência precisa do teste do caso real (a fonte cresceu desde a última vez), não só do de reexecução idêntica · evidence: `src-tauri/src/history/import/mod.rs:381` · features: shell-history-import

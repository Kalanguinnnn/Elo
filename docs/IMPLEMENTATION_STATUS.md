# Estado da implementação

Atualizado em 2026-10-30. Ambiente de validação: Windows 11 Pro, Rust 1.99.1, Node 24.18,
Git 2.55 (Windows), MSVC 14.51, WebView2 156.

## Milestones

| Milestone | Estado | Evidência |
|---|---|---|
| **M1 — Fundação e núcleo** | ✅ Concluído | Testes `snapshot.rs` e `sync.rs::full_flow_*`; demonstração ao vivo com dois Agents reais. |
| **M2 — Automação e aplicativo completo** | ✅ Concluído no Windows | 47 testes Rust e 4 de UI; app de release iniciado com o Agent via IPC; fluxo A→B→A sem comandos Git. |
| **M3 — Estabilização e distribuição** | 🟡 Parcial | Instaladores Windows gerados localmente; CI e release escritos, mas **ainda não executados** (repositório sem remoto GitHub). Linux não executado. |

## Recursos

| Recurso | Estado | Validação |
|---|---|---|
| Snapshot com índice isolado (staging/HEAD/branch preservados) | ✅ | Índice byte a byte idêntico; status, HEAD e branch inalterados (`capture_preserves_*`). |
| Arquivos modificados, novos, ignorados, excluídos, renomeados, staging parcial | ✅ | `capture_preserves_*`, `full_flow_*`. |
| Deduplicação, origem, branch e horário dos checkpoints | ✅ | `engine_dedups_*`; trailers `Continuity-*`. |
| Transporte entre clones sem force push | ✅ | Refs imutáveis; `full_flow_*`; `main` remota inalterada. |
| Aplicação protegida e merge de três vias | ✅ | `divergent_edits_*`, `keep_both_*`, `concurrent_edit_during_apply_*`. |
| Commits locais não enviados (fast-forward de HEAD) | ✅ | `commits_made_on_a_fast_forward_b_branch`. |
| Troca de branch com confirmação explícita | ✅ | `branch_switch_requires_explicit_confirmation`. |
| Branch divergente nunca resetada ("apenas arquivos") | ✅ | `diverged_branch_is_never_reset_*`. |
| Arquivo ignorado local não sobrescrito (backup se substituído) | ✅ | `local_ignored_file_is_not_overwritten`. |
| Confiança, IDE e período sem edições para aplicação automática | ✅ | `auto_apply_only_when_trusted_safe_and_clean`. A detecção de IDE é heurística. |
| Conflitos lado a lado e preservação da versão não escolhida | ✅ | `divergent_edits_*` (`.git/continuity/preserved`). UI implementada. |
| Fila offline persistente com backoff, inclusive após reinício | ✅ | `offline_queue_retries_*`. |
| Operação interrompida detectada após reinício | ✅ | `interrupted_operation_is_reported_after_restart`. |
| Concorrência (checkpoints simultâneos) | ✅ | `concurrent_checkpoints_are_serialised`. |
| Segurança de caminhos (traversal, `.git`, symlinks) | ✅ | `unsafe_paths_*`, testes de `paths.rs` (o teste de symlink roda só em Unix). |
| Integridade de binários e recuperação em outra pasta | ✅ | `binary_files_roundtrip_*`, verificação por hash. |
| Recuperação individual com checkpoint de segurança | ✅ | `restore_individual_file_*`. |
| Conversão de EOL (`autocrlf=true`) | ✅ | `eol_conversion_*`. |
| Detecção de segredos (bloqueio e confirmação) | ✅ | `secrets_block_or_require_confirmation_*` + testes unitários. |
| Retenção progressiva (local e remota, protegendo o necessário) | ✅ | `retention_*` + testes unitários. |
| Modo de commits Git automáticos | ✅ | `autocommit_mode_*` (staging parcial adiado; rejeição sem force). |
| Sincronizar e encerrar | ✅ | `sync_and_shutdown_*`; demonstração ao vivo. |
| Agent independente: IPC, watcher, scheduler, instância única | ✅ | `crates/agent/tests/agent.rs`; o app inicia o Agent sozinho. |
| Identidade persistente de dispositivo e renomeação | ✅ | `migrates_and_persists`, demonstração. |
| SQLite com migrações | ✅ | `db.rs` testes. |
| Token no cofre do SO e askpass restrito ao github.com | 🟡 | Lógica testada (`askpass_only_answers_github`). **Não testado contra o GitHub real** (requer credenciais do usuário). |
| GitHub: validar token, listar e criar repositórios | 🟡 | Implementado; **não executado** contra a API real nesta sessão. |
| Login GitHub pelo navegador (device flow) | 🟡 | Implementado; exige o client id de um OAuth App registrado (decisão/serviço externo). Configurável em Configurações. |
| Interface desktop (10 telas) | 🟡 | Compila (tsc estrito), build do Vite OK, app abre e chama o Agent. **Inspeção visual tela a tela não realizada.** |
| Animações e modo desempenho | ✅ | Transições de página, entrada escalonada de listas e feedback de hover/clique só com `opacity`/`transform`; **Ajustes → Aparência → Modo desempenho** desliga tudo (`data-motion="off"`), salvo por computador e aplicado antes da primeira pintura. Testes `motion.test.ts`; verificado no app (transição capturada a 40 ms) e no webview (`animationName: none` e transições 0 s com o modo ligado, inclusive após recarregar). |
| Instaladores Windows (NSIS e MSI) | ✅ | Gerados localmente em `target/release/bundle`. A instalação via instalador não foi testada; as builds não são assinadas (por requisito). |
| Pacotes Linux (deb, rpm, AppImage) | ⏳ | Configurados no `release.yml`; dependem da execução no CI. |
| CI (Windows/Linux) e release em rascunho | ⏳ | Workflows escritos; não executados (sem remoto GitHub). |

## Testes executados (última execução)

- `cargo test`: core 22 unitários + 6 snapshot + 18 sync; agent 1 → **47 passaram, 0 falharam**.
- `cargo clippy --workspace --all-targets -- -D warnings`: limpo (inclui o crate Tauri).
- `npm test` (vitest): 4 passaram; `npm run build`: OK.
- `npx tauri build --bundles nsis,msi`: OK.
- Demonstração ao vivo: dois Agents (`CONTINUITY_DATA_DIR` isolados) + CLI, fluxo A→B (com staging) → `sync-and-quit` → A.

## Problemas encontrados e corrigidos

- Deadlock: o guard do mutex do SQLite ficava vivo em `match`/closure (`sync_and_shutdown`, `process_queue`). Foi corrigido e coberto por testes; a regra foi registrada no AGENTS.md.
- Fixture de teste clonava com o `autocrlf` do sistema; os testes agora fixam a configuração e há um teste dedicado a EOL.

## Limitações conhecidas

- Submódulos (gitlinks) não são sincronizados; são reportados como ignorados.
- Restaurar ou aplicar muitos arquivos executa um processo `git` por arquivo, o que é lento em projetos enormes (ver ROADMAP).
- Em Linux, repositórios muito grandes podem esgotar o limite de inotify. O Agent registra o erro e passa a depender só da reconciliação periódica.
- A varredura de segredos cobre o estado publicado, não cada commit intermediário não enviado.
- `git log --all` exibe os commits de checkpoint (eles ficam em `refs/continuity/*`).
- O nome do dispositivo (hostname) vai nos metadados dos checkpoints enviados ao remoto; pode ser alterado.

## Próximas ações

1. Publicar o repositório no GitHub e rodar o CI (valida Linux e os pacotes .deb/.rpm/AppImage).
2. Revisão visual da UI no app e teste do instalador em uma máquina limpa.
3. Validar o fluxo com um repositório GitHub real (token pessoal) e, se desejado, registrar um OAuth App.
4. Decidir a licença (MIT ou Apache-2.0) e a marca.

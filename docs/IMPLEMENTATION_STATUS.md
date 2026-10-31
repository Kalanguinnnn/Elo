# Estado da implementação

Atualizado em 2026-10-31 (versão 0.1.1). Ambiente de validação: Windows 11 Pro, Rust 1.99.1, Node 24.18,
Git 2.55 (Windows), MSVC 14.51, WebView2 156.

## Milestones

| Milestone | Estado | Evidência |
|---|---|---|
| **M1 — Fundação e núcleo** | ✅ Concluído | Testes `snapshot.rs` e `sync.rs::full_flow_*`; demonstração ao vivo com dois Agents reais. |
| **M2 — Automação e aplicativo completo** | ✅ Concluído | Testes Rust e de UI (ver abaixo) verdes no Windows e no Linux; app de release iniciado com o Agent via IPC; fluxo A→B→A sem comandos Git. |
| **M3 — Estabilização e distribuição** | 🟡 Quase concluído | CI verde no GitHub em Windows e Linux a cada push; licença MIT; instaladores Windows gerados localmente e release 0.1.0 em rascunho pelo workflow (Windows + Linux). Falta o teste com dois computadores reais e a instalação em máquina limpa. |

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
| Token no cofre do SO e askpass restrito ao github.com | ✅ | `askpass_only_answers_github`; em uso real: login do GitHub feito pelo app e checkpoints deste repositório enviados ao GitHub (`refs/continuity/checkpoints/*` no remoto). |
| GitHub: validar token, listar e criar repositórios | 🟡 | Validação do token e conta conectada exercitadas em uso real; listar/criar repositório não foram exercitados contra a API real. |
| Login GitHub pelo navegador (device flow) | ✅ | Usado em uso real (conta conectada em Ajustes). O client id do OAuth App é configurável. |
| Interface desktop (10 telas) | ✅ | tsc estrito, build do Vite, testes da lógica de status (`status.test.ts`, `format.test.ts`, `motion.test.ts`) e revisão visual das telas no app em 2026-10-31. |
| Animações e modo desempenho | ✅ | Transições de página, entrada escalonada de listas e feedback de hover/clique só com `opacity`/`transform`; **Ajustes → Aparência → Modo desempenho** desliga tudo (`data-motion="off"`), salvo por computador e aplicado antes da primeira pintura. Testes `motion.test.ts`; verificado no app (transição capturada a 40 ms) e no webview (`animationName: none` e transições 0 s com o modo ligado, inclusive após recarregar). |
| Terceira auditoria de segurança | ✅ | 4 achados corrigidos (configuração Git do projeto não confiável, credenciais em URL, permissões do perfil no `--direct`, actions fixadas por SHA). Ver [AUDITORIA_SEGURANCA_3.md](../AUDITORIA_SEGURANCA_3.md#status-das-correções). |
| Instaladores Windows (NSIS e MSI) | ✅ | Gerados localmente em `target/release/bundle` e pelo workflow de release. A instalação em máquina limpa não foi testada; as builds não são assinadas (por requisito). |
| Pacotes Linux (deb, rpm, AppImage) | 🟡 | Gerados pelo workflow de release (tag `v0.1.0`, release em rascunho). Instalação em uma distribuição real não testada. |
| CI (Windows/Linux) e release em rascunho | ✅ | CI verde a cada push em `main`; actions fixadas por SHA; release em rascunho, nunca publicado automaticamente. |
| Licença | ✅ | MIT (`LICENSE`), declarada nos manifestos Cargo/npm e no instalador. |

## Testes executados (última execução)

- `cargo test`: **71 passaram no Windows** (core: 35 unitários, 6 snapshot, 26 sync, 3 política de workflows; agent: 1) e 72 no Linux (+1 só Unix), 0 falharam.
- `cargo clippy --workspace --all-targets -- -D warnings`: limpo (inclui o crate Tauri).
- `npm test` (vitest): 12 passaram; `npm run build`: OK.
- CI no GitHub (Windows e Linux): verde; no Linux roda também o teste de permissões do perfil (só Unix).
- Demonstração ao vivo: dois Agents (`CONTINUITY_DATA_DIR` isolados) + CLI, fluxo A→B (com staging) → `sync-and-quit` → A.

## Problemas encontrados e corrigidos

- Deadlock: o guard do mutex do SQLite ficava vivo em `match`/closure (`sync_and_shutdown`, `process_queue`). Foi corrigido e coberto por testes; a regra foi registrada no AGENTS.md.
- Fixture de teste clonava com o `autocrlf` do sistema; os testes agora fixam a configuração e há um teste dedicado a EOL.
- Revisão visual (2026-10-31): a tela Ajustes mostrava "Conectado" com um token do GitHub já revogado, e o push falhava só com a mensagem do Git. Agora `github.status` confere o token ao vivo (`token_valid`), Ajustes avisa para reconectar e a fila diz o que fazer (`rejected_github_login_names_the_fix`).
- Um teste continha um token falso literal, e o próprio Elo, sincronizando este repositório, colocou o checkpoint em quarentena. O teste agora gera o valor em tempo de execução, como os demais.
- Uso real (2026-10-31): o GitHub recusou um checkpoint que alterava `.github/workflows`, porque o login pedia só o escopo `repo`. O Elo mostrava apenas "failed to push some refs". Agora o login pede `repo workflow`, e o erro de push traz o motivo de cada ref recusada, com instrução para reconectar (`push_rejection_keeps_the_remote_reason`).
- Leitura em lote com `cat-file --batch --filters` revertida: o Git informa o tamanho sem conversão de EOL, e o teste `eol_conversion_*` detectou a divergência.

## Limitações conhecidas

- Submódulos (gitlinks) não são sincronizados; são reportados como ignorados.
- Restaurar ou aplicar muitos arquivos executa um processo `git` por arquivo, o que é lento em projetos enormes. A leitura em lote foi tentada e revertida: no Git 2.55, `cat-file --batch --filters` informa no cabeçalho o tamanho **sem** conversão de fim de linha e envia o conteúdo **convertido**, então não é possível separar os arquivos com segurança (o teste `eol_conversion_*` detectou isso).
- Em Linux, repositórios muito grandes podem esgotar o limite de inotify. O Agent registra o erro e passa a depender só da reconciliação periódica.
- A varredura de segredos cobre o estado publicado, não cada commit intermediário não enviado.
- `git log --all` exibe os commits de checkpoint (eles ficam em `refs/continuity/*`).
- O nome do dispositivo (hostname) vai nos metadados dos checkpoints enviados ao remoto; pode ser alterado.
- Projetos cujo `.git/config` define filtros ou helpers (inclusive `git lfs install --local`) ficam pausados até essas chaves irem para a configuração global; a mensagem indica o comando. `core.sshCommand` do projeto é ignorado (use `~/.ssh/config`).
- Nomes internos mantêm o nome antigo do projeto: binários `continuity`/`continuity-agent`, variáveis `CONTINUITY_*` e refs `refs/continuity/*` (mudar as refs exigiria migração dos remotos).

## Próximas ações

1. Testar o fluxo completo com **dois computadores reais** usando o GitHub (instalar a 0.1.0 no segundo PC, clonar e aplicar um checkpoint).
2. Instalar os pacotes em máquinas limpas (Windows sem ferramentas de desenvolvimento; Ubuntu/Fedora) e então publicar o rascunho do release.
3. Leitura em lote para aplicar/restaurar projetos enormes, contornando o comportamento do `cat-file --batch --filters` (por exemplo, com índice temporário + `checkout-index --prefix`).
4. Dividir `engine.rs` (≈1.770 linhas) por área: projetos, checkpoints, transporte, aplicação, GitHub.

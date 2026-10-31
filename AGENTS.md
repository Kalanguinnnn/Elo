# AGENTS.md — guia para quem (pessoa ou agente) altera este código

## Mapa

| Caminho | Conteúdo |
|---|---|
| `crates/core` | Regras de negócio. Sem dependência de UI. |
| `crates/core/src/snapshot.rs` | Captura com índice isolado e criação de checkpoints. |
| `crates/core/src/apply.rs` | Plano e execução da aplicação protegida. |
| `crates/core/src/engine.rs` | Orquestração, linhagem, fila, retenção e GitHub. |
| `crates/core/src/api.rs` | Tabela de métodos IPC (cada controle da UI corresponde a um método). |
| `crates/agent` | Processo de segundo plano: watcher, scheduler, IPC. |
| `crates/cli` | Binário `continuity`. |
| `apps/desktop` | Tauri 2 + React. `src-tauri` só faz proxy para o Agent. |

## Invariantes (não quebre)

1. Criar um checkpoint **nunca** escreve `.git/index`, HEAD ou refs de branch. Use
   `GIT_INDEX_FILE` apontando para uma cópia.
2. Push de checkpoint nunca usa `--force` ou refspec com `+`. As refs de checkpoint são imutáveis.
3. Nada é escrito no worktree sem: checkpoint de segurança, plano sem bloqueios e sem
   conflitos não resolvidos, e reverificação do hash de cada alvo (`verify_expectations`).
4. Todo caminho vindo de árvores Git passa por `paths::validate_rel_path` e
   `check_no_symlink_ancestors`.
5. Git é chamado apenas via `git::Git::run` (vetor de argumentos, sem shell). A configuração do
   repositório não é confiável: não remova as sobreposições de `run_raw` (fsmonitor, hooks,
   `GIT_ALLOW_PROTOCOL`) e, ao usar um subcomando novo, só o inclua em `SAFE` de
   `may_run_repo_programs` se ele nunca puder iniciar um programa configurado.
6. Tokens só existem no cofre do SO e no pipe do `GIT_ASKPASS`; nunca em logs, argv ou URLs.
7. Não segure `Engine::db()` enquanto chama outro método que também o bloqueia. Associe o
   resultado a uma variável antes de usar `match` ou `for` (o guard vive pelo bloco inteiro).

## Testes

- `cargo test` cria repositórios e remotos bare temporários. Nunca use remotos reais.
- Todo comportamento de sincronização novo precisa de um teste em `crates/core/tests/sync.rs`.
- `cargo clippy --all-targets -- -D warnings` deve passar.
- Não declare um recurso pronto sem teste executado; atualize `docs/IMPLEMENTATION_STATUS.md`.

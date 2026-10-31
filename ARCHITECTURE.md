# Arquitetura — Elo

O Elo mantém o estado de trabalho de um projeto Git (inclusive alterações não
commitadas) sincronizado entre computadores **sem tocar no staging, no HEAD ou na
branch** durante a captura, e **sem force push**.

## Visão geral

```
┌──────────────────────────┐      IPC local (named pipe / unix socket,
│ apps/desktop (Tauri 2)   │      JSON por linha + prova HMAC mútua)
│  React + TS + Vite (UI)  │ ───────────────────────────────┐
│  src-tauri: proxy IPC    │                                │
└──────────────────────────┘                                ▼
┌──────────────────────────┐              ┌──────────────────────────────────┐
│ crates/cli (`continuity`)│ ── IPC ────▶ │ crates/agent (`continuity-agent`)│
└──────────────────────────┘              │  servidor IPC · watcher (notify) │
                                          │  scheduler · worker da fila      │
                                          └───────────────┬──────────────────┘
                                                          │ chamadas Rust
                                          ┌───────────────▼──────────────────┐
                                          │ crates/core (`continuity-core`)  │
                                          │  engine · snapshot · transport   │
                                          │  apply · restore · secrets       │
                                          │  retention · db (SQLite) · ipc   │
                                          │  github · credentials · ide      │
                                          └───────────────┬──────────────────┘
                                                          │ std::process::Command (sem shell)
                                                          ▼
                                                     Git CLI ⇄ remoto Git
```

* **`continuity-core`** não depende de interface gráfica. Toda regra de negócio está
  aqui e é testada com repositórios temporários.
* **`continuity-agent`** é o processo de segundo plano. Continua rodando quando a
  janela é fechada. Uma única instância por usuário (lock de arquivo).
* **Desktop** é apenas cliente: cada controle da UI corresponde a um método IPC real.
  Se o Agent não estiver rodando, o app o inicia (binário sidecar).
* **CLI** é um segundo cliente, útil para automação e diagnóstico.

## Snapshot Engine (índice isolado)

Captura (`core::snapshot::capture`):

1. Recusa se existir `index.lock` (outra operação Git em andamento).
2. Copia `.git/index` para `.git/continuity/index-<rand>.tmp`.
3. `GIT_INDEX_FILE=<cópia> git write-tree` → árvore do **staging** atual.
4. `GIT_INDEX_FILE=<cópia> git add -A --ignore-errors` → inclui modificações,
   arquivos não rastreados (respeitando `.gitignore`) e exclusões.
5. `git write-tree` → árvore do **diretório de trabalho**.
6. Remove a cópia. O índice real, o HEAD e a branch nunca são escritos.

O checkpoint é um commit criado com `git commit-tree` (não executa hooks, não assina):

* commit de índice: `tree = staging`, `parent = HEAD`;
* commit de checkpoint: `tree = worktree`, `parents = [HEAD, commit de índice]`;
* mensagem com trailers `Continuity-*` (id, dispositivo, branch, HEAD, origem,
  horário, linhagem).

Cada checkpoint ganha uma referência **imutável**:
`refs/continuity/checkpoints/<device-uuid>/<id>` onde `id = <unix_ms 13 dígitos>-<hex6>`
(ordenável por tempo). Como as refs nunca são movidas, push e fetch nunca precisam de
`--force`. Capturas idênticas à cabeça de linhagem (mesma árvore, HEAD, branch e
staging) são descartadas.

## Linhagem e aplicação protegida

Cada dispositivo mantém, por projeto, uma **cabeça de linhagem**: o último checkpoint
que representa seu estado (criado por ele ou aplicado a partir de outro). O trailer
`Continuity-Parents` registra a linhagem; o SQLite guarda o grafo.

Para aplicar o checkpoint `T` de outro dispositivo (`core::apply`):

1. Cria um **checkpoint de segurança** do estado local (`ours`).
2. Base = ancestral comum de linhagem (LCA); sem LCA, a árvore do `merge-base` dos
   HEADs; sem isso, árvore vazia.
3. Merge de três vias **por arquivo**: alterado só em `theirs` → aplica; só em
   `ours` → preserva; nos dois e diferente → **conflito** (nada é escrito até o
   usuário resolver lado a lado).
4. Antes de escrever, re-verifica o hash de cada arquivo alvo (proteção contra
   edição concorrente). Arquivos ignorados localmente existentes nunca são
   sobrescritos.
5. Escrita atômica (temp + rename), conteúdo via `git cat-file --filters` (respeita
   EOL/filtros), validação de caminho (sem `..`, absolutos, `.git`, nomes reservados
   do Windows, componentes symlink) e verificação de integridade por
   `git hash-object` após a escrita.
6. HEAD: fast-forward da mesma branch é permitido com CAS (`update-ref new old`);
   troca de branch exige `allow_branch_switch`; divergência nunca é resolvida por
   reset — oferece-se "aplicar apenas arquivos".
7. Staging: transferido (`read-tree` do índice remoto) apenas se o staging local
   estava limpo.

Aplicação **automática** só ocorre quando: projeto confiável (padrão: desligado),
fast-forward sem alterações locais, sem conflitos, mesma branch, nenhum editor/IDE
conhecido em execução e nenhum evento de arquivo recente. Na dúvida → ação manual.

## Transporte

* Push: `git push --porcelain <remote> ref:ref ...` (sem `+`, sem `--force`).
* Fetch: `git fetch --no-tags <remote> refs/continuity/checkpoints/*:refs/continuity/checkpoints/*`.
* GitHub HTTPS: o token fica no cofre do sistema (`keyring`: Windows Credential
  Manager / Secret Service). O Git o obtém via `GIT_ASKPASS` apontando para o próprio
  binário do Elo — o token nunca vai para disco, argv ou URL.
* Operações de rede têm timeout; falhas transitórias entram na **fila persistente**
  (SQLite) com backoff exponencial.

## Persistência (SQLite, WAL)

Tabelas: `meta`, `device`, `devices`, `projects`, `checkpoints`, `queue`, `activity`,
`operations` (diário de operações para recuperação após reinício), `conflicts`,
`conflict_files`, `secret_findings`, `secret_allow`, `settings`. Migrações versionadas
em `core::db`.

## Segurança

* Comandos Git sempre via `Command::new("git").args([...])` — nunca shell.
* Varredura de segredos antes de cada push (regras bloqueantes e de alerta).
* Checkpoints com segredos bloqueantes ficam em quarentena local (nunca enviados).
* Nada é enviado a serviços próprios: o único destino é o remoto Git do usuário (e a
  API do GitHub, quando o usuário conecta a conta).

## Retenção progressiva

Por dispositivo: tudo nas últimas 24 h, o mais recente por dia por 30 dias, o mais
recente por semana por 12 semanas, mínimo de 20. Nunca remove: cabeça de linhagem,
checkpoints não enviados, referenciados por conflitos/operações, último por branch.
Refs do próprio dispositivo são removidas do remoto (push de deleção, não force).

## Justificativas

* **Git CLI em vez de libgit2**: comportamento idêntico ao Git do usuário (filtros,
  EOL, hooks de credencial, protocolo v2), sem divergência de implementação.
* **Refs imutáveis em namespace próprio**: elimina force push e não aparece em
  `git branch`/`git log` normais.
* **Merge por arquivo próprio**: controle total sobre o que é escrito no disco e
  verificação de integridade, em vez de `git checkout`/`read-tree -u` no worktree.
* **Agent separado**: sincronização continua sem a janela aberta; UI e CLI
  compartilham o mesmo núcleo.
* **Threads em vez de async**: carga de trabalho pequena, dominada por processos Git;
  menos dependências e mais previsibilidade.

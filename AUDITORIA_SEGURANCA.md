# Security Review: APP GITHUB ELo

## Scope

Auditoria estática do código-fonte, configuração, scripts, workflows, testes e documentação do repositório atual.

- Scan mode: repository
- Target kind: git_revision
- Target ID: target_sha256_a77e3d521de35fee2b824579d6968e75c9e2c069778d48f6afcd9bf8af478593
- Revision: a8127848996a2a58f1eef090c5cd39ff10ef92db
- Inventory strategy: repository
- Included paths: .
- Excluded paths: none
- Artifacts reviewed: Código Rust em crates/core, crates/agent, crates/cli e src-tauri, UI React/Tauri e scripts de build, Testes de sincronização e Agent, Política SECURITY.md, documentação e workflows

Limitations and exclusions:
- Dependências de terceiros em lockfiles não foram auditadas.
- ACLs efetivas do arquivo agent.token e do named pipe no Windows dependem da instalação real.
- Explorabilidade da janela de corrida por symlink não foi reproduzida dinamicamente.
- Excluded apps/desktop/src-tauri/icons/\*\*: Ícones binários empacotados, sem lógica executável própria.
- Excluded Cargo.lock: Metadados de dependências; análise de vulnerabilidades de terceiros fora desta revisão de código-fonte.
- Excluded apps/desktop/package-lock.json: Metadados de dependências; análise de vulnerabilidades de terceiros fora desta revisão de código-fonte.
- Excluded apps/desktop/src-tauri/icons/\*\*: Ícones binários empacotados; não contêm lógica executável analisável neste fluxo estático.
- Excluded Cargo.lock; apps/desktop/package-lock.json: Metadados de versões de dependências; não foi feita análise de vulnerabilidades de terceiros.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 5 |
| Severity mix | high: 3, medium: 1, low: 1 |
| Confidence mix | high: 4, medium: 1 |
| Coverage | complete |
| Validation mode | Revisão estática de fluxo de dados e controles |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

Elo sincroniza worktrees Git entre computadores por checkpoints em refs próprias do remoto do usuário. O app Tauri encaminha chamadas ao Agent local; a CLI usa IPC ou abre o Engine diretamente com --direct. O Agent monitora arquivos, agenda checkpoints, fetch, aplicação automática e retenção. Evidência: apps/desktop/src-tauri/src/lib.rs:16-70; crates/cli/src/main.rs:73-115; crates/agent/src/lib.rs:113-171,206-286.

### Assets

- Integridade do worktree, staging, HEAD, refs e cópias preservadas em diretório Git privado. Evidência: crates/core/src/snapshot.rs:54-79; crates/core/src/apply.rs:485-542; crates/core/src/git.rs:244-248.
- Conteúdo de projetos enviado ao remoto Git, incluindo alterações não commitadas e refs de branch no autocommit opcional. Evidência: crates/core/src/snapshot.rs:54-79; crates/core/src/transport.rs:43-74; crates/core/src/autocommit.rs:44-50.
- Identidade do dispositivo, projetos, fila, histórico e alertas no SQLite; token IPC em agent.token; credencial GitHub no keyring. Evidência: crates/core/src/engine.rs:222-225; crates/core/src/ipc.rs:59-69,82-106; crates/core/src/credentials.rs:9-35.

### Trust Boundaries

- WebView → comando Tauri agent_call → IPC → dispatch do Engine. O proxy aceita qualquer método e o servidor verifica token antes do handler. Evidência: apps/desktop/src-tauri/src/lib.rs:47-70; crates/core/src/ipc.rs:167-199; crates/core/src/api.rs:48-56.
- CLI → Agent por IPC ou CLI --direct → Engine sem IPC, usando os mesmos dados e keyring. Evidência: crates/cli/src/main.rs:37-47,89-115; crates/core/src/engine.rs:222-225.
- Worktree → índice temporário Git → objetos e refs de checkpoint. A captura copia .git/index e usa GIT_INDEX_FILE. Evidência: crates/core/src/snapshot.rs:54-79,105-118; crates/core/src/git.rs:244-248.
- Remoto Git → refs importadas → plano → escrita local. O plano valida caminhos e links; a execução verifica alvos e o Engine cria checkpoint de segurança. Evidência: crates/core/src/engine.rs:903-985; crates/core/src/apply.rs:162-225,413-495; crates/core/src/paths.rs:15-90.
- Projeto confiável → aplicação automática. A confiança inicia falsa; exige plano elegível e sondagem de editor. Evidência: crates/core/src/engine.rs:373-377,903-951,1452-1471.
- Engine → Git CLI/remoto. Usa argv, limpa variáveis Git selecionadas, desativa prompts e não força refs de checkpoint. Evidência: crates/core/src/git.rs:17-29,106-128; crates/core/src/transport.rs:43-74.
- Engine → GitHub API e keyring. OAuth device flow, consulta e criação de repositórios; ASKPASS responde apenas para prompts HTTPS github.com. Evidência: crates/core/src/api.rs:227-260; crates/core/src/engine.rs:1525-1545; crates/core/src/credentials.rs:17-35,69-88; crates/core/src/github.rs:47-96.

### Attacker Capabilities

- Colaborador ou remoto Git hostil pode fornecer árvores, blobs, caminhos, metadados e refs buscadas pelo Agent; não se presume controle do usuário local ou keyring. Evidência: crates/core/src/transport.rs:94-104; crates/core/src/engine.rs:743-759; crates/core/src/apply.rs:162-225.
- Outro usuário local pode tentar acessar pipe/socket e agent.token; a efetividade das ACLs Windows não está estabelecida no código. Evidência: crates/core/src/ipc.rs:59-69,82-110,130-136,167-199.
- Código executando na conta do operador pode usar CLI --direct e já compartilha a autoridade local. Evidência: crates/cli/src/main.rs:89-115; crates/core/src/engine.rs:222-225.

### Security Objectives

- Checkpoint não modifica índice real, HEAD ou branch; aplicação requer checkpoint, plano seguro e reverificação. Evidência: crates/core/src/snapshot.rs:54-79; crates/core/src/engine.rs:903-985; crates/core/src/apply.rs:485-495.
- Caminhos remotos permanecem dentro do projeto, sem atravessar symlinks/reparse points; alvos de symlink ficam contidos. Evidência: crates/core/src/apply.rs:175-192; crates/core/src/paths.rs:15-90,112-135.
- Push de checkpoint não força refs; alertas bloqueantes impedem envio e avisos exigem aprovação. Evidência: crates/core/src/transport.rs:43-74; crates/core/src/engine.rs:664-728,1248-1265.
- Token GitHub fica no keyring, entregue ao Git por ASKPASS somente para github.com; não deve aparecer em logs, argv ou URL. Evidência: crates/core/src/credentials.rs:17-35,69-88; crates/core/src/transport.rs:23-32.
- Aplicação automática desativada por padrão e condicionada a projeto confiável, plano elegível e editor seguro. Evidência: crates/core/src/engine.rs:373-377,903-951,1452-1471.

### Assumptions

- Solicitação do usuário: auditar todo o repositório e produzir relatório Markdown.
- SECURITY.md afirma IPC por usuário e agent.token legível apenas pelo usuário. O código restringe permissões explicitamente apenas em Unix; ACL Windows depende do SO e da biblioteca. Evidência: crates/core/src/ipc.rs:82-110,130-136.
- A documentação de git.rs diz remover GIT_\* herdadas; a implementação lista variáveis específicas. Evidência: crates/core/src/git.rs:1-29.
- CONTINUITY_DATA_DIR pode substituir o diretório padrão no Agent e CLI direta; isolamento depende das permissões do local. Evidência: crates/core/src/lib.rs:27-42; crates/agent/src/main.rs:23-39; crates/cli/src/main.rs:93-101.
- SQLite e agent.log/agent.lock ficam em \<data_dir\>; índice temporário e preserved/\<op_id\> ficam em \<git_dir\>/continuity; remotos recebem refs/continuity/checkpoints; credencial GitHub está no keyring. Evidência: crates/core/src/db.rs:346-360; crates/agent/src/logger.rs:21-25; crates/core/src/git.rs:244-248; crates/core/src/transport.rs:43-104; crates/core/src/credentials.rs:9-35.
- Constatado na auditoria: o envio de checkpoint varre apenas a árvore do worktree, embora a ref inclua um commit pai com a árvore do índice; o push de branch no autocommit segue caminho separado. Evidência: crates/core/src/snapshot.rs:110-116; crates/core/src/engine.rs:690,728,1365-1375.
- Constatado na auditoria: metadados de dispositivo/linhagem de refs de checkpoint são aceitos por formato, sem autenticação da origem antes de aplicação automática em projeto confiável. Evidência: crates/core/src/checkpoint.rs:120-154; crates/core/src/engine.rs:759-798,851-866,1452-1471.

## Findings

| Finding | Severity | Confidence | Detailed write-up |
| --- | --- | --- | --- |
| [Push automático da branch contorna a varredura de segredos](#finding-1) | high | high | inline below |
| [Segredo somente no staging é enviado no checkpoint](#finding-2) | high | high | inline below |
| [Checkpoint remoto forjado pode ser aplicado automaticamente](#finding-3) | high | high | inline below |
| [Limites silenciosos deixam credenciais fora da varredura](#finding-4) | medium | high | inline below |
| [Troca concorrente de diretório pode desviar escrita para fora do projeto](#finding-5) | low | medium | inline below |

### Confidence Scale

| Label | Meaning |
| --- | --- |
| high | Direct evidence supports the finding with no material unresolved blocker. |
| medium | Evidence supports a plausible issue, but material runtime or reachability proof remains. |
| low | Evidence is incomplete and the item is retained only for explicit follow-up. |

<a id="finding-1"></a>

### [1] Push automático da branch contorna a varredura de segredos

| Field | Value |
| --- | --- |
| Severity | high |
| Confidence | high |
| Confidence rationale | Enfileiramento e execução de push_branch inspecionados; não há chamada de scanner nesse caminho. |
| Category | Push de conteúdo sem controle de segredos |
| CWE | CWE-200 |
| Affected lines | crates/core/src/engine.rs:1365, crates/core/src/autocommit.rs:40, crates/core/src/autocommit.rs:45, crates/core/src/engine.rs:1401 |

#### Summary

Com autocommit e auto_push ativados, o Agent cria um commit e enfileira push_branch. Esse caminho envia a branch sem chamar o scanner que protege o push de checkpoints.

#### Root Cause

O caminho push_branch foi implementado sem a verificação de segredos exigida pelo caminho push_pending.

**Enfileiramento da branch** — `crates/core/src/autocommit.rs:45`

O envio da branch é separado do checkpoint.

```rust
engine.db().enqueue(&p.id, "push_branch", &branch)?;
```

**Execução sem scanner** — `crates/core/src/engine.rs:1374`

A branch é enviada diretamente.

```rust
transport::push_branch(&git, &p.remote, branch, &auth)?;
```

**Scanner em outro caminho** — `crates/core/src/engine.rs:690`

O scanner só ocorre em push_pending.

```rust
let findings = secrets::scan_trees(&git, &base, &c.tree_oid)?;
```

#### Validation

auto_commit enfileira push_branch quando autocommit/auto_push estão ativos; process_queue chama push_branch diretamente. A quarentena de checkpoint não impede essa operação.

Validation method: Revisão estática do fluxo de fila

Counterevidence and remaining uncertainty:
- Autocommit vem desativado por padrão
- O push de checkpoint possui scanner

#### Dataflow

Worktree → autocommit → fila push_branch → transport::push_branch → remoto, sem secrets::scan_trees.

#### Reachability

Requer autocommit e auto_push habilitados e remoto configurado.

- **Attacker:** Leitor do remoto

- **Entry point:** Conteúdo local capturado pelo autocommit

- **Sink:** Branch remota

#### Severity

**High** — Uma credencial em arquivo capturado pelo autocommit pode chegar ao remoto mesmo quando o push do checkpoint correspondente seria bloqueado.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Executar a varredura bloqueante de todos os blobs novos antes de enfileirar ou executar push_branch; não liberar a branch enquanto houver achados. Adicionar teste de sincronização em crates/core/tests/sync.rs.

<a id="finding-2"></a>

### [2] Segredo somente no staging é enviado no checkpoint

| Field | Value |
| --- | --- |
| Severity | high |
| Confidence | high |
| Confidence rationale | A árvore do índice é pai do checkpoint e a chamada de varredura recebe apenas c.tree_oid; relação verificada no código. |
| Category | Varredura incompleta dos objetos publicados |
| CWE | CWE-200 |
| Affected lines | crates/core/src/engine.rs:690, crates/core/src/snapshot.rs:110, crates/core/src/snapshot.rs:115, crates/core/src/engine.rs:728 |

#### Summary

O checkpoint inclui um commit pai com a árvore do índice, mas o push só examina a árvore do worktree. Um segredo removido do worktree e ainda presente no staging é transmitido sem alerta.

#### Root Cause

A varredura usa somente a árvore de trabalho, apesar de a ref do checkpoint incluir a árvore do índice como commit pai.

**Commit do índice** — `crates/core/src/snapshot.rs:110`

A árvore staged é armazenada como commit.

```rust
let index_commit = commit_tree(git, &state.index_tree, &head_parent, &idx_msg)?;
```

**Pai do checkpoint** — `crates/core/src/snapshot.rs:115`

O push da ref alcança o objeto do índice.

```rust
parents.push(&index_commit);
```

**Varredura só da árvore de trabalho** — `crates/core/src/engine.rs:690`

A árvore staged não é verificada.

```rust
let findings = secrets::scan_trees(&git, &base, &c.tree_oid)?;
```

#### Validation

Se o usuário fizer staging de um segredo e removê-lo só do worktree, o índice ainda contém o blob. A varredura de c.tree_oid não encontra o blob, e o push da ref envia seu commit pai.

Validation method: Revisão estática de objetos alcançáveis

Counterevidence and remaining uncertainty:
- A árvore de trabalho é examinada
- Achados bloqueantes na árvore examinada impedem o push

#### Dataflow

Arquivo staged → index_tree → index_commit pai → ref de checkpoint → remoto; scanner examina somente worktree tree.

#### Reachability

Requer segredo no índice e ausência dele no worktree no momento do checkpoint; leitores do remoto podem obter o objeto publicado.

- **Attacker:** Leitor do remoto

- **Entry point:** Conteúdo staged

- **Sink:** Remoto Git

#### Severity

**High** — O objeto publicado pode conter uma credencial ativa que o usuário tentou retirar do arquivo de trabalho; o controle de quarentena não a examina.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Varrer todos os blobs novos alcançáveis pela ref, inclusive a árvore do índice, antes de enviar; bloquear o checkpoint ao detectar segredo. Cobrir o caso em crates/core/tests/sync.rs.

<a id="finding-3"></a>

### [3] Checkpoint remoto forjado pode ser aplicado automaticamente

| Field | Value |
| --- | --- |
| Severity | high |
| Confidence | high |
| Confidence rationale | Fluxo de fetch, importação, cálculo de base, elegibilidade e execução conferido no código; não foi executada prova dinâmica. |
| Category | Autenticidade de checkpoint remoto não verificada |
| CWE | CWE-345 |
| Affected lines | crates/core/src/engine.rs:759, crates/core/src/engine.rs:851, crates/core/src/engine.rs:1452, crates/core/src/apply.rs:102, crates/core/src/checkpoint.rs:130 |

#### Summary

Quem pode criar refs de checkpoint no remoto pode forjar identidade e linhagem de outro dispositivo. Em um projeto marcado como confiável e sem alterações locais, o Agent pode escrever conteúdo arbitrário no worktree sem confirmação.

#### Root Cause

O importador confia no nome da ref e em trailers de commit controlados pelo remoto para identificar o dispositivo e a linhagem; não verifica assinatura, chave aprovada ou parentesco Git antes da aplicação automática.

**Metadados aceitos por formato** — `crates/core/src/checkpoint.rs:130`

A origem do dispositivo é validada sintaticamente, sem autenticação.

```rust
if !is_valid_id(&id) || !is_valid_device_id(&device_id) {
```

**Linhas de parentes declaradas no commit** — `crates/core/src/checkpoint.rs:141`

O remetente controla os IDs de linhagem usados depois como base.

```rust
let parents = field("Parents").split_whitespace().filter(|p| is_valid_id(p)).map(String::from).collect();
```

**Base selecionada por parentes declarados** — `crates/core/src/engine.rs:855`

A linhagem importada influencia diretamente a base do plano.

```rust
for anc in self.lineage_ancestors(&p.id, &theirs.id)? {
```

#### Validation

Um colaborador pode publicar uma nova ref com UUID estrangeiro, indicar o checkpoint local atual como Parent, manter Branch/Head compatíveis e apontar para uma árvore alterada. O plano vê base igual ao estado local, sem conflito, e auto_eligible permite a escrita. Confiança local, editor ocioso, checkpoint de segurança e hash dos alvos continuam sendo pré-condições/controles.

Validation method: Revisão estática de fluxo

Counterevidence and remaining uncertainty:
- trusted inicia falso
- IDE e plano precisam estar seguros
- checkpoint de segurança e hash são verificados

#### Dataflow

Ref e commit forjados → fetch → importação de metadados → base_tree por Parent → plan/auto_eligible → apply::execute.

#### Reachability

Alcançável por escritor de refs de checkpoint do remoto quando o projeto da vítima está confiável, não pausado, o editor está ocioso e o worktree está limpo.

- **Attacker:** Colaborador ou credencial comprometida com escrita no remoto

- **Entry point:** refs/continuity/checkpoints/\*

- **Sink:** worktree e staging locais

Preconditions:
- Escrita no remoto
- Projeto marcado como confiável
- Plano sem conflitos

#### Severity

**High** — O atacante obtém escrita automática no worktree de outro usuário, inclusive de arquivos de código, desde que tenha escrita nas refs do remoto e o projeto esteja marcado como confiável; o caminho dispensa revisão humana.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Autenticar checkpoints com assinaturas vinculadas a dispositivo aprovado, commit, árvore, índice, branch, HEAD e pais; exigir linhagem direta verificada para aplicação automática. Adicionar teste de sincronização com ref e trailers forjados.

<a id="finding-4"></a>

### [4] Limites silenciosos deixam credenciais fora da varredura

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Ambas as condições fazem continue sem achado; fluxo posterior inclui o checkpoint no push. |
| Category | Conteúdo não inspecionado tratado como seguro |
| CWE | CWE-200 |
| Affected lines | crates/core/src/secrets.rs:127, crates/core/src/secrets.rs:177, crates/core/src/engine.rs:720 |

#### Summary

O scanner ignora linhas acima de 4.000 bytes e blobs acima de 1 MiB. Entre 1 MiB e 50 MiB não há sequer aviso de arquivo grande; o push pode prosseguir com credenciais reconhecíveis.

#### Root Cause

Limites de desempenho descartam conteúdo sem produzir estado 'não examinado' ou bloqueio de envio.

**Linha longa ignorada** — `crates/core/src/secrets.rs:127`

Uma linha inteira deixa de ser analisada.

```rust
if line.len() > 4000 {
    continue; // minified bundles: too noisy
}
```

**Blob grande ignorado** — `crates/core/src/secrets.rs:177`

Blob entre o limite de varredura e o de alerta não gera achado.

```rust
if size > MAX_SCAN_BYTES {
    continue;
}
```

**Envio sem achados** — `crates/core/src/engine.rs:720`

Resultado vazio permite o push.

```rust
to_push.push(c.clone());
```

#### Validation

Um token após o byte 4.000 de linha única ou em arquivo genérico acima de 1 MiB não é submetido às regex; se não houver outro achado, a ref é enviada.

Validation method: Revisão estática de condições do scanner

Counterevidence and remaining uncertainty:
- Nomes de arquivo sensíveis ainda geram achados
- Arquivos a partir de 50 MiB geram aviso

#### Dataflow

Arquivo local → scan_trees com continue → findings vazio → push_pending → remoto.

#### Reachability

Requer credencial em linha \>4.000 bytes ou blob \>1 MiB e \<=50 MiB, sem regra de nome que bloqueie.

- **Attacker:** Leitor do remoto

- **Entry point:** Conteúdo local capturado

- **Sink:** Ref remota

#### Severity

**Medium** — Arquivos JSON/minificados e arquivos textuais entre 1 e 50 MiB são plausíveis; o vazamento exige que contenham segredo e sejam enviados.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Varrer conteúdo em fluxo e linhas longas em janelas sobrepostas; quando não puder inspecionar, exigir revisão explícita ou bloquear o push. Adicionar testes de sincronização com token após 4.000 bytes e após 1 MiB.

<a id="finding-5"></a>

### [5] Troca concorrente de diretório pode desviar escrita para fora do projeto

| Field | Value |
| --- | --- |
| Severity | low |
| Confidence | medium |
| Confidence rationale | A separação entre verificação e operações de caminho é visível no código; não houve reprodução dinâmica nem comprovação de diretório compartilhado na instalação padrão. |
| Category | Condição de corrida entre validação de caminho e escrita |
| CWE | CWE-367 |
| Affected lines | crates/core/src/paths.rs:139, crates/core/src/paths.rs:150, crates/core/src/paths.rs:219 |

#### Summary

A verificação de ancestrais e a escrita usam caminhos em momentos separados. Em um projeto compartilhado, outro usuário com permissão para trocar um diretório pode substituí-lo por link entre a verificação e a abertura do arquivo temporário.

#### Root Cause

A implementação não mantém identificadores de diretório abertos entre a checagem de symlink/reparse point e a operação de escrita ou remoção.

**Ancestrais verificados por nome** — `crates/core/src/paths.rs:139`

A verificação termina antes da escrita.

```rust
check_no_symlink_ancestors(root, target)?;
```

**Abertura posterior por caminho** — `crates/core/src/paths.rs:150`

O caminho é resolvido novamente pelo SO após a verificação.

```rust
let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
```

#### Validation

Um segundo usuário com escrita em um ancestral do projeto pode tentar trocá-lo por symlink após a última verificação. O Agent resolve o caminho novamente com suas próprias permissões. O código verifica duas vezes e substitui symlink no alvo final, reduzindo mas não eliminando a janela.

Validation method: Revisão estática de sequência de operações

Limitations:
- Não houve teste de corrida ou medição de explorabilidade
- Depende de projeto compartilhado com permissões entre usuários

#### Dataflow

Diretório do projeto controlado por outro usuário → troca por symlink após check_no_symlink_ancestors → open/rename/remove por caminho → destino externo.

#### Reachability

Condicional a diretório compartilhado e troca concorrente precisa.

- **Attacker:** Outro usuário com escrita em diretório do projeto mas sem escrita no destino externo

- **Entry point:** Ancestral de arquivo aplicado

- **Sink:** Arquivo fora do projeto

#### Severity

**Low** — Exige projeto gravável por outro usuário e acerto de uma janela curta de concorrência; pode fazer o Agent escrever com privilégios do operador fora do projeto.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Executar criação, renomeação e remoção relativas a diretórios abertos e verificados, sem seguir links ou reparse points; testar troca concorrente de ancestral.

## Reviewed Surfaces

| Surface | Risk Area | Outcome | Notes |
| --- | --- | --- | --- |
| Refs remotas, metadados de checkpoint e aplicação automática | Integridade do worktree | Reported | Checkpoint forjado pode ser importado e aplicado em projeto confiável; controles de plano e checkpoint de segurança reduzem perda local mas não autenticam o remetente. crates/core/src/engine.rs:759-798,851-866,1452-1471. |
| Captura, varredura de segredos e push Git | Confidencialidade de credenciais | Reported | O commit pai do índice, o push de branch do autocommit e limites silenciosos de tamanho escapam da varredura. crates/core/src/snapshot.rs:110-116; crates/core/src/secrets.rs:127,177; crates/core/src/engine.rs:690,1365-1375. |
| Validação de caminhos e escrita de arquivos recebidos | Integridade do sistema de arquivos | Reported | Validação de caminhos e links presente, mas verificação de ancestrais e escrita não são operação atômica em projetos compartilhados. crates/core/src/paths.rs:77-95,135-152. |
| IPC local, token, credenciais e execução Git | Autoridade local | No issue found | Nenhum ataque entre usuários confirmado na instalação padrão. Token verificado por requisição, keyring e ASKPASS restringem acesso; ACL Windows efetiva permanece questão de implantação. crates/core/src/ipc.rs:82-110,167-199; crates/core/src/credentials.rs:17-35,69-88; crates/core/src/git.rs:106-128. |
| UI React, proxy Tauri e abertura de URLs | Execução de conteúdo na interface | No issue found | Dados remotos exibidos como texto React; nenhum innerHTML/eval. Capability do opener restringe URLs a domínios esperados. apps/desktop/src/ui.tsx:57-59,92; apps/desktop/src-tauri/capabilities/default.json:11-14. |
| CLI, Agent, testes, scripts, CI e documentação | Execução e publicação | No issue found | Fluxos de execução e testes analisados; nenhum ataque adicional confirmado. Testes de sincronização existentes não cobrem refs forjadas, staging isolado ou limites do scanner. crates/core/tests/sync.rs:302-395; .github/workflows/ci.yml:3-65. |

## Open Questions And Follow Up

- Verificar as ACLs efetivas de agent.token e do named pipe em uma instalação Windows padrão e em diretórios de dados personalizados.

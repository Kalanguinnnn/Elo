# Security Review: APP GITHUB ELo

## Scope

Auditoria estática do estado de trabalho atual no commit 53bef4e. Foram revisados código, testes, configuração, workflows e documentação textual. Modelo de ameaça gerado durante a auditoria. Assets gráficos binários não contêm lógica executável e ficaram fora da revisão de código.

- Scan mode: repository
- Target kind: git_worktree
- Target ID: target_sha256_a77e3d521de35fee2b824579d6968e75c9e2c069778d48f6afcd9bf8af478593
- Revision: 53bef4eacb799644ce6a065f46cb258964e0c70b
- Snapshot digest: codex-security-snapshot/v1:sha256:d03b9f892805f95d94de7a601109847972a0df18c2367eb0ec6aa8aa417f0b2a
- Inventory strategy: repository
- Included paths: .
- Excluded paths: none
- Artifacts reviewed: SECURITY.md, AGENTS.md, Cargo.toml, Cargo.lock, package.json, package-lock.json, .github/workflows, crates/core/src, crates/agent/src, crates/cli/src, crates/core/tests, crates/agent/tests, apps/desktop/src, apps/desktop/src-tauri, docs, README.md, ROADMAP.md, CONTRIBUTING.md

Limitations and exclusions:
- Não foram executados testes nem reproduzidos comportamentos em runtime.
- Riscos locais dependem de ACLs/umask e compartilhamento do worktree.
- Risco do workflow depende de comprometimento externo.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 4 |
| Severity mix | medium: 2, low: 2 |
| Confidence mix | high: 4 |
| Coverage | complete |
| Validation mode | static source review |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

O Elo sincroniza checkpoints de worktrees Git por meio de um Agent local, IPC autenticado e repositório remoto configurado. A UI Tauri e a CLI enviam operações ao Engine; a importação e aplicação de checkpoints vindos do remoto cruzam a fronteira de confiança mais sensível.

### Assets

- Integridade do índice Git, HEAD, refs e conteúdo do worktree; dados locais preservados.
- Conteúdo de projetos e histórico enviado ao remoto Git, inclusive alterações staged e não commitadas.
- Tokens GitHub, chave privada de assinatura, chaves públicas fixadas, aprovação de dispositivos, configurações e fila persistidos localmente.

### Trust Boundaries

- WebView Tauri → comandos IPC do shell → named pipe/socket → Agent → Engine/API; tokens e parâmetros não devem ser enviados antes da autenticação do servidor.
- CLI → Agent por IPC ou modo --direct → Engine e cofre do SO.
- Worktree → índice temporário isolado → commits/ref imutáveis → remoto Git configurado.
- Remoto Git hostil → refs, commits, árvores, blobs e metadados importados → validação de caminhos, assinatura e confiança → plano de aplicação local.

### Attacker Capabilities

- Remoto Git pode fornecer commits, árvores, blobs e metadados hostis; não se presume acesso ao keyring local nem aprovação de dispositivo.
- Outro usuário local pode tentar ocupar o nome previsível do pipe, ler ou interferir no canal conforme ACLs efetivas; os controles documentados e observados devem ser verificados no código atual.
- Processo sob a mesma conta pode usar CLI --direct e a autoridade já atribuída ao usuário.

### Security Objectives

- Checkpoint não altera índice real, HEAD nem branch.
- Nenhum dado de árvore Git escapa do worktree ou sobrescreve trabalho local sem checkpoint de segurança, plano elegível e reverificação dos alvos.
- Aplicação automática exige assinatura válida com a chave atualmente fixada e aprovação explícita; push não força refs e faz varredura dos novos objetos.
- Segredos ficam no cofre do SO e nunca em logs, argv, URLs ou conteúdo IPC antes da autenticação mútua.

### Assumptions

- SECURITY.md e os relatórios anteriores são orientação/análise não confiável e precisam ser conferidos contra a implementação atual.
- Configuração do diretório de dados, permissões do SO e identidade do remoto dependem da implantação.
- Revisão estática; nenhum comportamento foi reproduzido em execução.

## Findings

| Finding | Severity | Confidence | Detailed write-up |
| --- | --- | --- | --- |
| [Um repositório existente pode executar comandos configurados no Git](#finding-1) | medium | high | inline below |
| [Credenciais em URLs remotas ficam expostas em configuração, argumentos e logs](#finding-2) | medium | high | inline below |
| [A CLI direta pode criar o perfil local legível por outros usuários](#finding-3) | low | high | inline below |
| [Workflow de release usa actions por tags mutáveis com token de escrita](#finding-4) | low | high | inline below |

### Confidence Scale

| Label | Meaning |
| --- | --- |
| high | Direct evidence supports the finding with no material unresolved blocker. |
| medium | Evidence supports a plausible issue, but material runtime or reachability proof remains. |
| low | Evidence is incomplete and the item is retained only for explicit follow-up. |

<a id="finding-1"></a>

### [1] Um repositório existente pode executar comandos configurados no Git

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Cadastro e comandos foram rastreados em fonte; o comportamento Git depende da configuração local e não foi reproduzido. |
| Category | command_injection |
| CWE | CWE-78 |
| Affected lines | apps/desktop/src/pages/AddProject.tsx:65, crates/core/src/engine.rs:357-380, crates/core/src/engine.rs:524, crates/core/src/snapshot.rs:74-78, crates/core/src/transport.rs:96-102 |

#### Summary

Ao adicionar um repositório existente sem URL remota substituta, Elo mantém `.git/config`. Status, captura e fetch automáticos executam Git com essa configuração; `core.fsmonitor`, filtros locais e `ext::` podem iniciar comandos sob a conta do Agent.

#### Root Cause

O cadastro confia no `.git/config` existente se `remote_url` não é informado. Depois, o Agent executa Git sem isolar helpers externos, filtros ou protocolo de transporte local.

**UI aceita remote_url ausente** — `apps/desktop/src/pages/AddProject.tsx:65`

Campo vazio vira null.

```typescript
const p = await call<Project>("project.add", { path: info?.workdir ?? path, init_git: initGit, remote_url: remoteUrl || null });
```

**A config Git existente permanece** — `crates/core/src/engine.rs:357-380`

Somente URL não vazia substitui o remote atual.

```rust
let git = match Git::open(path) { Ok(g) => g, ... };
if let Some(url) = opts.remote_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) { self.set_remote_url(&git, "origin", url)?; }
```

**Status executa Git na pasta** — `crates/core/src/engine.rs:524-525`

Config local pode habilitar core.fsmonitor.

```rust
git.run(os_args!["status", "--porcelain=v1", "-z", "--untracked-files=normal"], RunOpts::default())
```

**Captura usa git add** — `crates/core/src/snapshot.rs:74-78`

Filtros configurados são acionáveis por atributos durante add.

```rust
git.run(os_args!["add", "-A", "--ignore-errors", "--", "."], env(RunOpts::default().config("advice.addEmbeddedRepo", "false").config("core.safecrlf", "false").ok_code(1)))?;
```

**Fetch delega resolução de URL ao Git** — `crates/core/src/transport.rs:96-102`

Valida o nome, mas usa URL/protocolo do config local.

```rust
pub fn fetch_checkpoints(git: &Git, remote: &str, auth: &NetAuth) -> Result<()> {
 validate_remote_name(remote)?;
 let spec = format!("{REF_PREFIX}*:{REF_PREFIX}*");
 git.run(os_args!["fetch", "--no-tags", "--no-write-fetch-head", "--no-auto-gc", "--quiet", remote, spec], auth.apply(RunOpts::default()))?;
 Ok(())
}
```

#### Validation

Confirmei que o cadastro conserva o remote existente e as operações automáticas chamam status, add e fetch sem restringir helpers da configuração local.

Validation method: revisão estática e rastreamento independente

**UI aceita remote_url ausente** — `apps/desktop/src/pages/AddProject.tsx:65`

Campo vazio vira null.

```typescript
const p = await call<Project>("project.add", { path: info?.workdir ?? path, init_git: initGit, remote_url: remoteUrl || null });
```

**A config Git existente permanece** — `crates/core/src/engine.rs:357-380`

Somente URL não vazia substitui o remote atual.

```rust
let git = match Git::open(path) { Ok(g) => g, ... };
if let Some(url) = opts.remote_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) { self.set_remote_url(&git, "origin", url)?; }
```

**Status executa Git na pasta** — `crates/core/src/engine.rs:524-525`

Config local pode habilitar core.fsmonitor.

```rust
git.run(os_args!["status", "--porcelain=v1", "-z", "--untracked-files=normal"], RunOpts::default())
```

**Captura usa git add** — `crates/core/src/snapshot.rs:74-78`

Filtros configurados são acionáveis por atributos durante add.

```rust
git.run(os_args!["add", "-A", "--ignore-errors", "--", "."], env(RunOpts::default().config("advice.addEmbeddedRepo", "false").config("core.safecrlf", "false").ok_code(1)))?;
```

**Fetch delega resolução de URL ao Git** — `crates/core/src/transport.rs:96-102`

Valida o nome, mas usa URL/protocolo do config local.

```rust
pub fn fetch_checkpoints(git: &Git, remote: &str, auth: &NetAuth) -> Result<()> {
 validate_remote_name(remote)?;
 let spec = format!("{REF_PREFIX}*:{REF_PREFIX}*");
 git.run(os_args!["fetch", "--no-tags", "--no-write-fetch-head", "--no-auto-gc", "--quiet", remote, spec], auth.apply(RunOpts::default()))?;
 Ok(())
}
```

Assertions:
- A validação de URL cobre clone/set explícitos, não a URL existente antes do fetch.
- status/add não desabilitam fsmonitor ou filtros externos.

Limitations:
- Exige escrita atacante em `.git/config`; conteúdo remoto sozinho não controla esse arquivo.
- Nenhuma execução dinâmica foi realizada.

#### Dataflow

config local atacante → project.add sem URL → configuração preservada → Git status/add/fetch → subprocesso

- **Source:** `.git/config` gravável por colaborador

- **Sink:** Helper iniciado pelo Git

- **Outcome:** execução sob usuário do Agent

**UI aceita remote_url ausente** — `apps/desktop/src/pages/AddProject.tsx:65`

Campo vazio vira null.

```typescript
const p = await call<Project>("project.add", { path: info?.workdir ?? path, init_git: initGit, remote_url: remoteUrl || null });
```

**A config Git existente permanece** — `crates/core/src/engine.rs:357-380`

Somente URL não vazia substitui o remote atual.

```rust
let git = match Git::open(path) { Ok(g) => g, ... };
if let Some(url) = opts.remote_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) { self.set_remote_url(&git, "origin", url)?; }
```

**Status executa Git na pasta** — `crates/core/src/engine.rs:524-525`

Config local pode habilitar core.fsmonitor.

```rust
git.run(os_args!["status", "--porcelain=v1", "-z", "--untracked-files=normal"], RunOpts::default())
```

**Captura usa git add** — `crates/core/src/snapshot.rs:74-78`

Filtros configurados são acionáveis por atributos durante add.

```rust
git.run(os_args!["add", "-A", "--ignore-errors", "--", "."], env(RunOpts::default().config("advice.addEmbeddedRepo", "false").config("core.safecrlf", "false").ok_code(1)))?;
```

**Fetch delega resolução de URL ao Git** — `crates/core/src/transport.rs:96-102`

Valida o nome, mas usa URL/protocolo do config local.

```rust
pub fn fetch_checkpoints(git: &Git, remote: &str, auth: &NetAuth) -> Result<()> {
 validate_remote_name(remote)?;
 let spec = format!("{REF_PREFIX}*:{REF_PREFIX}*");
 git.run(os_args!["fetch", "--no-tags", "--no-write-fetch-head", "--no-auto-gc", "--quiet", remote, spec], auth.apply(RunOpts::default()))?;
 Ok(())
}
```

#### Reachability

Exige escrita no config local de projeto ativo; a execução posterior é automática.

- **Attacker:** Colaborador ou processo local com escrita em worktree compartilhado

- **Entry point:** Cadastro de repo existente

- **Outcome:** Comando executado sob o Agent

Limitations:
- URLs informadas pelo usuário e clones são validados inicialmente; o achado cobre config preexistente.

#### Severity

**Medium** — Pode ocorrer execução de código com os privilégios do usuário. Requer escrita em `.git/config` de um worktree e cadastro pelo usuário, o que limita a probabilidade.

Aumenta em ambientes com worktrees compartilhados graváveis por colaboradores menos confiáveis; diminui em diretórios privados.

Impact assessment:
- **Level:** high
- **Why:** O comando pode ler e alterar dados acessíveis à conta e usar credenciais disponíveis.

Likelihood assessment:
- **Level:** medium
- **Why:** Worktree compartilhado gravável limita o cenário, mas o restante do fluxo ocorre normalmente.

#### Remediation

Execute Git com configuração restrita, desative `core.fsmonitor` e filtros não aprovados e valide protocolo remoto efetivo antes de cada operação de rede.

Tests:
- Repo com core.fsmonitor externo não deve executar comando ao consultar status.
- Captura deve bloquear filtro externo configurado.
- Fetch não deve invocar remote-helper ext.

Preventive controls:
- Não confiar em configuração Git de worktrees não confiáveis.
- Aplicar allowlist à URL efetiva em fetch, push e probe.

<a id="finding-2"></a>

### [2] Credenciais em URLs remotas ficam expostas em configuração, argumentos e logs

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Validação, encaminhamento ao Git, log e status estão explícitos no código. |
| Category | sensitive_data_exposure |
| CWE | CWE-532 |
| Affected lines | crates/core/src/transport.rs:122-133, crates/core/src/engine.rs:410-421, crates/core/src/engine.rs:426-431, crates/core/src/engine.rs:518-524 |

#### Summary

A validação aceita URLs com `user:token@host`. O Elo passa a URL ao Git, que a persiste em `.git/config`; a troca de remote também grava o valor literal no log e o status o retorna pela API.

#### Root Cause

O validador não rejeita userinfo. O setter passa a URL ao Git e registra o valor literal no log; o status também retorna a URL do config.

**Validador permite userinfo** — `crates/core/src/transport.rs:122-133`

Rejeita controles e alguns transports, mas aceita `https://user:token@host/...`.

```rust
pub fn validate_remote_url(url: &str) -> Result<()> {
 let u = url.trim();
 let lower = u.to_ascii_lowercase();
 if u.is_empty() || u.starts_with("-") || u.chars().any(|c| c.is_control()) { return Err(...); }
 ...
 Ok(())
}
```

**URL inteira vai ao Git** — `crates/core/src/engine.rs:410-421`

URL não é redigida antes do argumento e configuração Git.

```rust
transport::validate_remote_url(url)?;
...
git.run(os_args!["remote", "add", name, url], RunOpts::default())?;
```

**URL completa persiste no log** — `crates/core/src/engine.rs:426-431`

A mensagem persiste userinfo literal.

```rust
self.set_remote_url(&git, &p.remote, url)?;
self.log(Some(project_id), "info", "remote_set", &format!("Remoto {} configurado: {url}", p.remote));
```

**Status retorna URL do config** — `crates/core/src/engine.rs:518-524`

ProjectStatus inclui o URL sem redaction.

```rust
st.remote_url = git.config_get(&format!("remote.{}.url", p.remote))?;
```

#### Validation

Confirmei que URL com userinfo passa pela validação, vira argumento Git, é gravada em config, aparece no log da troca e retorna no status.

Validation method: revisão estática do fluxo de credenciais

**Validador permite userinfo** — `crates/core/src/transport.rs:122-133`

Rejeita controles e alguns transports, mas aceita `https://user:token@host/...`.

```rust
pub fn validate_remote_url(url: &str) -> Result<()> {
 let u = url.trim();
 let lower = u.to_ascii_lowercase();
 if u.is_empty() || u.starts_with("-") || u.chars().any(|c| c.is_control()) { return Err(...); }
 ...
 Ok(())
}
```

**URL inteira vai ao Git** — `crates/core/src/engine.rs:410-421`

URL não é redigida antes do argumento e configuração Git.

```rust
transport::validate_remote_url(url)?;
...
git.run(os_args!["remote", "add", name, url], RunOpts::default())?;
```

**URL completa persiste no log** — `crates/core/src/engine.rs:426-431`

A mensagem persiste userinfo literal.

```rust
self.set_remote_url(&git, &p.remote, url)?;
self.log(Some(project_id), "info", "remote_set", &format!("Remoto {} configurado: {url}", p.remote));
```

**Status retorna URL do config** — `crates/core/src/engine.rs:518-524`

ProjectStatus inclui o URL sem redaction.

```rust
st.remote_url = git.config_get(&format!("remote.{}.url", p.remote))?;
```

Assertions:
- Não há rejeição nem redaction de userinfo.
- O log interpola URL integral.
- Status copia URL literal.

Limitations:
- Não há evidência de exfiltração remota; a exposição é local.

#### Dataflow

URL digitada → validador permissivo → git remote set-url/add → config/log/status

- **Source:** URL com credenciais

- **Sink:** Configuração Git, log SQLite e resposta API

- **Outcome:** segredo fora do keyring

**Validador permite userinfo** — `crates/core/src/transport.rs:122-133`

Rejeita controles e alguns transports, mas aceita `https://user:token@host/...`.

```rust
pub fn validate_remote_url(url: &str) -> Result<()> {
 let u = url.trim();
 let lower = u.to_ascii_lowercase();
 if u.is_empty() || u.starts_with("-") || u.chars().any(|c| c.is_control()) { return Err(...); }
 ...
 Ok(())
}
```

**URL inteira vai ao Git** — `crates/core/src/engine.rs:410-421`

URL não é redigida antes do argumento e configuração Git.

```rust
transport::validate_remote_url(url)?;
...
git.run(os_args!["remote", "add", name, url], RunOpts::default())?;
```

**URL completa persiste no log** — `crates/core/src/engine.rs:426-431`

A mensagem persiste userinfo literal.

```rust
self.set_remote_url(&git, &p.remote, url)?;
self.log(Some(project_id), "info", "remote_set", &format!("Remoto {} configurado: {url}", p.remote));
```

**Status retorna URL do config** — `crates/core/src/engine.rs:518-524`

ProjectStatus inclui o URL sem redaction.

```rust
st.remote_url = git.config_get(&format!("remote.{}.url", p.remote))?;
```

#### Reachability

Configuração de remote aceita o URL diretamente sem exigir keyring/askpass.

- **Attacker:** Usuário que fornece URL com credencial; leitor local dos artefatos

- **Entry point:** Cadastro ou atualização de remote

- **Outcome:** credencial persistida

Limitations:
- URLs sem userinfo não são afetadas.

#### Severity

**Medium** — Credenciais que podem autorizar acesso remoto são copiadas para armazenamento e argumentos de processo. Requer userinfo real e acesso local ao projeto, log ou argumentos.

Aumenta para tokens de escrita e projetos compartilhados; URLs sem segredo não são afetadas.

Impact assessment:
- **Level:** medium
- **Why:** Token com permissões pode autorizar acesso ao repositório.

Likelihood assessment:
- **Level:** medium
- **Why:** O URL é aceito sem alerta e seus valores persistem.

#### Remediation

Rejeite URLs com userinfo, use keyring/askpass para credenciais e mascare URLs em logs e status.

Tests:
- HTTPS URL com username/password deve ser recusada.
- Logs/status devem remover userinfo de URLs legadas.

Preventive controls:
- Redigir userinfo antes de log/serialização.
- Manter credenciais no keyring.

<a id="finding-3"></a>

### [3] A CLI direta pode criar o perfil local legível por outros usuários

| Field | Value |
| --- | --- |
| Severity | low |
| Confidence | high |
| Confidence rationale | CLI direta evita Agent; Engine/Db não definem permissões e Agent restringe explicitamente. |
| Category | security_misconfiguration |
| CWE | CWE-732 |
| Affected lines | crates/cli/src/main.rs:84-103, crates/core/src/engine.rs:229-232, crates/core/src/db.rs:359-365, crates/agent/src/lib.rs:79-82 |

#### Summary

`continuity --direct` abre Engine sem o startup do Agent, único lugar que chama `restrict_dir`. Com perfil novo em Linux, umask permissivo e diretórios-pai traversáveis, SQLite pode herdar permissões padrão e expor metadados locais.

#### Root Cause

A restrição do diretório só está no Agent. CLI direta abre Engine/Db, que apenas cria pasta e arquivo SQLite sem modo privado.

**--direct abre Engine diretamente** — `crates/cli/src/main.rs:84-103`

Caminho não inicia Agent.

```rust
let direct = args.first().is_some_and(|a| a == "--direct");
...
let backend = if direct { match Engine::open(EngineConfig { data_dir, ... }) { ... } }
```

**Engine cria pasta sem restrição** — `crates/core/src/engine.rs:229-232`

Não define modo privado.

```rust
pub fn open(cfg: EngineConfig) -> Result<Engine> {
 std::fs::create_dir_all(&cfg.data_dir)?;
 let db = Db::open(&cfg.data_dir.join("continuity.db"))?;
```

**SQLite usa criação padrão** — `crates/core/src/db.rs:359-365`

Arquivo é aberto sem modo restrito explícito.

```rust
pub fn open(path: &Path) -> Result<Db> {
 if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
 let conn = Connection::open(path)?;
 Self::init(conn)
}
```

**Restrição ocorre no startup do Agent** — `crates/agent/src/lib.rs:79-82`

CLI direta não passa por esta chamada.

```rust
pub fn start(opts: AgentOptions) -> Result<(Agent, Server)> {
 ipc::restrict_dir(&opts.data_dir);
 let engine = Arc::new(Engine::open(...)?);
```

#### Validation

Comparei o startup do Agent com `--direct`; a criação de data_dir/SQLite não define permissões restritas no caminho direto.

Validation method: comparação estática das rotas de startup

**--direct abre Engine diretamente** — `crates/cli/src/main.rs:84-103`

Caminho não inicia Agent.

```rust
let direct = args.first().is_some_and(|a| a == "--direct");
...
let backend = if direct { match Engine::open(EngineConfig { data_dir, ... }) { ... } }
```

**Engine cria pasta sem restrição** — `crates/core/src/engine.rs:229-232`

Não define modo privado.

```rust
pub fn open(cfg: EngineConfig) -> Result<Engine> {
 std::fs::create_dir_all(&cfg.data_dir)?;
 let db = Db::open(&cfg.data_dir.join("continuity.db"))?;
```

**SQLite usa criação padrão** — `crates/core/src/db.rs:359-365`

Arquivo é aberto sem modo restrito explícito.

```rust
pub fn open(path: &Path) -> Result<Db> {
 if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
 let conn = Connection::open(path)?;
 Self::init(conn)
}
```

**Restrição ocorre no startup do Agent** — `crates/agent/src/lib.rs:79-82`

CLI direta não passa por esta chamada.

```rust
pub fn start(opts: AgentOptions) -> Result<(Agent, Server)> {
 ipc::restrict_dir(&opts.data_dir);
 let engine = Arc::new(Engine::open(...)?);
```

Assertions:
- --direct chama Engine::open diretamente.
- Engine::open não chama restrict_dir.
- Db::open usa Connection::open.

Limitations:
- Umask e ACLs dos diretórios-pai não foram reproduzidos em Linux.

#### Dataflow

CLI direta → Engine::open → create_dir_all → SQLite open → leitura local

- **Source:** Opção --direct

- **Sink:** Perfil SQLite sem permissão privada

- **Outcome:** Leitura de metadados do Elo

**--direct abre Engine diretamente** — `crates/cli/src/main.rs:84-103`

Caminho não inicia Agent.

```rust
let direct = args.first().is_some_and(|a| a == "--direct");
...
let backend = if direct { match Engine::open(EngineConfig { data_dir, ... }) { ... } }
```

**Engine cria pasta sem restrição** — `crates/core/src/engine.rs:229-232`

Não define modo privado.

```rust
pub fn open(cfg: EngineConfig) -> Result<Engine> {
 std::fs::create_dir_all(&cfg.data_dir)?;
 let db = Db::open(&cfg.data_dir.join("continuity.db"))?;
```

**SQLite usa criação padrão** — `crates/core/src/db.rs:359-365`

Arquivo é aberto sem modo restrito explícito.

```rust
pub fn open(path: &Path) -> Result<Db> {
 if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
 let conn = Connection::open(path)?;
 Self::init(conn)
}
```

**Restrição ocorre no startup do Agent** — `crates/agent/src/lib.rs:79-82`

CLI direta não passa por esta chamada.

```rust
pub fn start(opts: AgentOptions) -> Result<(Agent, Server)> {
 ipc::restrict_dir(&opts.data_dir);
 let engine = Arc::new(Engine::open(...)?);
```

#### Reachability

Requer perfil novo/não restrito e ambiente Linux com ACL/umask permissivos.

- **Attacker:** Outro usuário local

- **Entry point:** Criação do perfil pela CLI

- **Outcome:** Acesso de leitura ao banco

Limitations:
- Exposição depende das permissões efetivas no sistema.

#### Severity

**Low** — Exposição de metadados exige modo opcional --direct, perfil não restringido e ACL/umask permissivos; chaves e token GitHub ficam no keyring.

Aumenta se o banco tiver credenciais em URL; diminui com home privado ou perfil previamente restrito.

Impact assessment:
- **Level:** low
- **Why:** Banco contém metadados/histórico, não a chave privada ou token que ficam no keyring.

Likelihood assessment:
- **Level:** medium
- **Why:** Modo é explícito e permissões variam por ambiente.

#### Remediation

Aplique permissões privadas na camada central de inicialização de Engine/Db antes de criar o banco.

Tests:
- Em Linux com perfil inexistente, Engine direto deve criar diretório privado e SQLite sem leitura para grupo/outros.

Preventive controls:
- Criar diretório e banco com modo restrito em todos os entrypoints.

<a id="finding-4"></a>

### [4] Workflow de release usa actions por tags mutáveis com token de escrita

| Field | Value |
| --- | --- |
| Severity | low |
| Confidence | high |
| Confidence rationale | Permissões, action refs e token estão definidos no workflow; comprometimento é condição hipotética. |
| Category | supply_chain |
| CWE | CWE-829 |
| Affected lines | .github/workflows/release.yml:10-12, .github/workflows/release.yml:25-48, .github/workflows/release.yml:48-55 |

#### Summary

O workflow de release concede `contents: write` e usa actions de terceiros por tags mutáveis, incluindo `tauri-apps/tauri-action@v0`. Se uma referência for comprometida ou redirecionada, código não revisado pode alterar releases e artefatos.

#### Root Cause

Actions são resolvidas por tags mutáveis no job com contents:write; a Tauri action também recebe GITHUB_TOKEN.

**Permissão de escrita** — `.github/workflows/release.yml:10-12`

Job pode alterar release.

```yaml
permissions:
  contents: write
```

**Actions por tags** — `.github/workflows/release.yml:25-48`

Tags não fixam revisão imutável.

```yaml
- uses: actions/checkout@v4
- uses: dtolnay/rust-toolchain@stable
- uses: Swatinem/rust-cache@v2
- uses: actions/setup-node@v4
...
- uses: tauri-apps/tauri-action@v0
```

**Token passado à action** — `.github/workflows/release.yml:48-55`

Action de release recebe token.

```yaml
- uses: tauri-apps/tauri-action@v0
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
  with:
    releaseDraft: true
```

#### Validation

Confirmei permissão de escrita, referências por tag e token passado à action Tauri.

Validation method: revisão estática do workflow

**Permissão de escrita** — `.github/workflows/release.yml:10-12`

Job pode alterar release.

```yaml
permissions:
  contents: write
```

**Actions por tags** — `.github/workflows/release.yml:25-48`

Tags não fixam revisão imutável.

```yaml
- uses: actions/checkout@v4
- uses: dtolnay/rust-toolchain@stable
- uses: Swatinem/rust-cache@v2
- uses: actions/setup-node@v4
...
- uses: tauri-apps/tauri-action@v0
```

**Token passado à action** — `.github/workflows/release.yml:48-55`

Action de release recebe token.

```yaml
- uses: tauri-apps/tauri-action@v0
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
  with:
    releaseDraft: true
```

Assertions:
- O job possui contents:write.
- Actions externas não são fixadas por SHA.
- Action de release recebe token.

Limitations:
- Nenhuma tag comprometida foi observada.

#### Dataflow

tag mutável → action → token contents:write → release/artefatos

- **Source:** Action third-party versionada por tag

- **Sink:** API GitHub de escrita

- **Outcome:** Integridade de release comprometida

**Permissão de escrita** — `.github/workflows/release.yml:10-12`

Job pode alterar release.

```yaml
permissions:
  contents: write
```

**Actions por tags** — `.github/workflows/release.yml:25-48`

Tags não fixam revisão imutável.

```yaml
- uses: actions/checkout@v4
- uses: dtolnay/rust-toolchain@stable
- uses: Swatinem/rust-cache@v2
- uses: actions/setup-node@v4
...
- uses: tauri-apps/tauri-action@v0
```

**Token passado à action** — `.github/workflows/release.yml:48-55`

Action de release recebe token.

```yaml
- uses: tauri-apps/tauri-action@v0
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
  with:
    releaseDraft: true
```

#### Reachability

Requer comprometimento de upstream/tag e execução do workflow.

- **Attacker:** Atacante supply chain

- **Entry point:** Tag v\* ou workflow_dispatch

- **Outcome:** Alteração de artefatos

Limitations:
- Release é criada como draft e requer revisão manual.

#### Severity

**Low** — Impacto possível em instaladores, mas exige comprometimento/retargeting externo e execução do release workflow. Rascunho depende de revisão manual, que não garante integridade.

Aumenta se houver publicação automática ou segredos adicionais; diminui com SHA imutável e permissões mínimas.

Impact assessment:
- **Level:** high
- **Why:** Instaladores podem ser adulterados.

Likelihood assessment:
- **Level:** low
- **Why:** Comprometimento da referência é pré-condição rara.

#### Remediation

Fixe actions externas por SHA completo revisado e reduza contents:write ao job necessário.

Tests:
- Validador do workflow deve rejeitar action refs sem SHA completo.
- Permissões de escrita devem estar apenas no job de release.

Preventive controls:
- Atualizar SHAs sob revisão.
- Usar privilégio mínimo para GITHUB_TOKEN.

## Reviewed Surfaces

| Surface | Risk Area | Outcome | Notes |
| --- | --- | --- | --- |
| IPC entre UI, CLI e Agent | Autenticação local e identidade do endpoint | No issue found | Handshake HMAC/nonce, DACL por usuário e checagem do SID do peer protegem o canal; DoS local de pipe ocupado está documentado como fora do escopo. |
| Confiança de dispositivo e aplicação protegida | Assinaturas, autorização e escrita no worktree | No issue found | Reset invalida flags verified e apply revalida chave atual; caminhos e escritas confinadas foram revisados. |
| Execução por configuração Git de projeto | Git helpers e config não confiáveis | Reported | Config existente é preservada no cadastro sem URL e usada em status, snapshot e fetch. |
| Segredos, OAuth GitHub e ASKPASS | Armazenamento e autenticação de credenciais | No issue found | Fluxos normais usam keyring/ASKPASS e endpoints/hosts restritos; nenhum envio a host arbitrário foi identificado. |
| Credenciais em URLs remotas | Persistência e log de segredos | Reported | Userinfo é aceito e copiado ao config Git, argumentos, logs e status. |
| Permissões do perfil CLI direto | Confidencialidade do banco local | Reported | `--direct` abre Engine sem `restrict_dir`; modo efetivo depende do sistema. |
| Workflow de release | Integridade de build e release | Reported | Tags mutáveis e contents:write permitem impacto se uma action externa for comprometida. |
| UI React/Tauri | XSS, opener, CSP e IPC | No issue found | Dados remotos renderizados como texto; não foram encontrados sinks HTML dinâmicos e opener é limitado por capability. |
| Sincronização, retenção e varredura de segredos | Git, estado local e remote | No issue found | Módulos core/Agent/CLI e testes de integração revisados estaticamente; testes não foram executados. |
| CI, dependências, instalação e documentação | Supply chain e deployment | No issue found | Cargo.lock usa crates.io e package-lock usa registry npm com integrity; workflows e docs revisados. |
| Credenciais em URLs remotas | Credential persistence and logging | Reported | URLs com userinfo passam na validação, são encaminhadas ao Git, registradas sem redaction e retornadas no status. |
| Permissões do perfil no modo CLI direto | Local data confidentiality | Reported | A rota `--direct` cria/abre perfil e SQLite sem chamar o `restrict_dir` do Agent; exposição varia conforme umask e permissões dos pais. |
| Workflow de release e ações de terceiros | Build and release supply chain | Reported | Workflow concede contents:write e executa actions por tags mutáveis na geração de release drafts. |

## Status das correções

Atualizado após a remediação. Cada correção tem teste automatizado.

| Achado | Status | Correção | Teste |
| --- | --- | --- | --- |
| [1] Repositório existente pode executar comandos configurados no Git | ✅ Corrigido | A configuração do próprio repositório deixou de ser confiável. **Em toda chamada do Git**: `-c core.fsmonitor=false`; `core.hooksPath` apontando para um arquivo (nenhum hook do projeto roda, inclusive `reference-transaction`); `GIT_ALLOW_PROTOCOL=file:http:https:ssh`, que bloqueia `ext::`/`fd::` e outros helpers mesmo via `url.*.insteadOf` e prevalece sobre `protocol.*.allow`. **Antes de comandos que podem iniciar programas configurados** (`status`, `add`, `hash-object`, `update-index`, `read-tree`, `cat-file --filters`, `fetch`, `push`, `ls-remote` e qualquer subcomando novo, por padrão), o Elo lê a configuração com escopo e recusa o projeto se o escopo local/worktree, ou arquivos incluídos por ele, definir `filter.*.clean/smudge/process`, `credential.*helper`, `core.askPass`, `core.alternateRefsCommand`, `remote.*.uploadpack/receivepack/vcs`. O erro `unsafe_repo_config` aparece no cadastro, no status ("Sincronização pausada por segurança") e na fila. As mesmas chaves no escopo global/sistema continuam valendo, por exemplo o Git LFS. **Antes de cada fetch/push/probe**, as URLs efetivas (`git remote get-url [--push] --all`, que aplica `insteadOf`) passam pela validação de transporte. | `audit3_finding1_project_git_config_cannot_run_programs` (fsmonitor, hook, filtro, helper de credencial no cadastro e `ext::` via `insteadOf`; cada caso confere que o `git` comum **executaria** o payload), `git::tests::repo_hazards_ignore_user_scopes_and_harmless_keys`, `git::tests::only_known_plumbing_skips_the_config_check` |
| [2] Credenciais em URLs remotas expostas em config, argumentos e logs | ✅ Corrigido | `validate_remote_url` recusa userinfo em URLs HTTP(S) (token sozinho também) e senha em `ssh://`. A mensagem orienta a conectar o GitHub em Ajustes e não repete o segredo. Vale para cadastro, troca de remoto e clone, então nada chega ao `.git/config`, ao argv ou ao log. URLs antigas já gravadas são mascaradas (`https://***@host`) no status, na inspeção de pasta, no log de troca de remoto e em toda mensagem de erro do Git (argumentos e stderr). | `audit3_finding2_credentials_in_urls_are_refused_and_never_shown`, `transport::tests::audit3_finding2_rejects_credentials_in_urls`, `util::tests::redacts_userinfo_in_urls` |
| [3] CLI direta pode criar perfil legível por outros usuários | ✅ Corrigido | `Engine::open` restringe a pasta de dados (`0700`) antes de abrir o banco. `Db::open` cria o arquivo já com `0600` e aperta um arquivo existente; o SQLite copia esse modo para `-wal`/`-shm`. Vale para Agent, `--direct` e testes. | `audit3_finding3_engine_creates_a_private_profile_on_every_entry_point` (Unix; executado no CI Linux) |
| [4] Workflow de release usa actions por tags mutáveis com token de escrita | ✅ Corrigido | Todas as actions dos dois workflows foram fixadas por SHA completo, com a versão em comentário (`checkout` v4.4.0, `rust-toolchain` stable, `rust-cache` v2.9.3, `setup-node` v4.4.0, `tauri-action` v0.6.2). O release passou a ter `contents: read` global e `contents: write` só no job `bundle`. | `workflow_policy::audit3_finding4_actions_are_pinned_by_full_sha`, `workflow_policy::audit3_finding4_write_permission_only_at_job_level` |

Verificação:

- Os testes novos não compilam contra o código anterior, porque usam o campo `unsafe_config`. Por isso cada proteção foi retirada isoladamente e o teste correspondente rodado de novo. Nos 6 casos o teste **falhou**, ou seja, nenhuma barreira é redundante:
  - sem `core.fsmonitor=false`, o fsmonitor do projeto roda;
  - sem `core.hooksPath`, o hook roda;
  - sem a checagem de configuração, o checkpoint executa o filtro;
  - sem a validação da URL efetiva e sem `GIT_ALLOW_PROTOCOL`, o `ext::` roda;
  - sem a rejeição de credenciais, a URL com token é aceita;
  - sem o mascaramento no status, o token aparece.
- Escopo dos testes: o do achado 3 só roda em Unix, porque no Windows o perfil fica em `%LOCALAPPDATA%` com ACL do usuário. Os payloads do achado 1 foram executados no Windows, pelo `sh` do Git for Windows.
- Riscos residuais:
  - `core.sshCommand` do projeto é ignorado, porque o Elo sempre define `GIT_SSH_COMMAND`. Se você usa uma chave SSH por repositório, configure-a no `~/.ssh/config`.
  - Projetos com `git lfs install --local` ou outros filtros/helpers locais precisam mover essas chaves para a configuração global para voltar a sincronizar.

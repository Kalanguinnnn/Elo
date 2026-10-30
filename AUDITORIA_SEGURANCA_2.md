# Security Review: APP GITHUB ELo

## Scope

Auditoria estática do estado atual do repositório, incluindo mudanças de assinatura/aprovação, IPC, UI, regras de sincronização, testes e documentação.

- Scan mode: repository
- Target kind: git_revision
- Target ID: target_sha256_a77e3d521de35fee2b824579d6968e75c9e2c069778d48f6afcd9bf8af478593
- Revision: 516a710d337b15cd538b6d42c761b37bd68f6023
- Inventory strategy: repository
- Included paths: .
- Excluded paths: none
- Artifacts reviewed: Código Rust do core, Agent e CLI, UI React e shell Tauri, Testes de sincronização e Agent, SECURITY.md, documentação, scripts e workflows

Limitations and exclusions:
- ACLs efetivas do pipe variam com Windows/instalação; a falha identificada é o cliente não autenticar o servidor antes de enviar.
- Não foram executados testes dinâmicos de Windows ou rotação de chave.
- Dependências de terceiros em lockfiles não foram auditadas.
- Excluded apps/desktop/src-tauri/icons/\*\*: Ícones binários empacotados sem lógica executável própria.
- Excluded Cargo.lock: Metadados de dependências; análise de vulnerabilidades de terceiros fora desta revisão de código-fonte.
- Excluded apps/desktop/package-lock.json: Metadados de dependências; análise de vulnerabilidades de terceiros fora desta revisão de código-fonte.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 2 |
| Severity mix | high: 1, medium: 1 |
| Confidence mix | high: 2 |
| Coverage | complete |
| Validation mode | Revisão estática de arquitetura, fluxo de dados e controles |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

Elo sincroniza worktrees Git por checkpoints em refs de um remoto configurado. O app Tauri encaminha chamadas ao Agent por IPC; CLI pode usar IPC ou Engine direto. O Agent agenda checkpoints, sincronização, aplicação e retenção. O código atual assina checkpoints, fixa chaves de dispositivos e exige aprovação para aplicação automática. Evidência: apps/desktop/src-tauri/src/lib.rs:16-70; crates/cli/src/main.rs:73-115; crates/agent/src/lib.rs:113-286; crates/core/src/engine.rs:803-908,1033-1043.

### Assets

- Integridade do worktree, staging, HEAD e refs, com cópias preservadas sob git_dir. Evidência: crates/core/src/snapshot.rs:54-79; crates/core/src/apply.rs:485-542.
- Conteúdo de projetos enviado ao remoto, incluindo checkpoints e branch opcional de autocommit. Evidência: crates/core/src/engine.rs:703-740,1499-1531.
- Identidade/chaves de dispositivo, projetos e fila no SQLite; token IPC em agent.token; credenciais e chave privada de assinatura no keyring. Evidência: crates/core/src/engine.rs:222-253; crates/core/src/ipc.rs:82-106; crates/core/src/signing.rs:13-35.

### Trust Boundaries

- WebView → Tauri agent_call → IPC → dispatch; servidor verifica token antes do handler. apps/desktop/src-tauri/src/lib.rs:46-70; crates/core/src/ipc.rs:167-199; crates/core/src/api.rs:44-56.
- CLI usa IPC ou --direct, que abre Engine/keyring diretamente. crates/cli/src/main.rs:37-47,89-115.
- Worktree → índice isolado → commits de checkpoint; captura usa cópia e GIT_INDEX_FILE. crates/core/src/snapshot.rs:54-79,107-126.
- Remoto → refs importadas → assinatura/chave fixada/aprovação → plano → aplicação; aplica após checkpoint de segurança e reverificação. crates/core/src/engine.rs:803-908,996-1101.
- Engine → Git/remoto; argv sem shell, push sem força, objetos novos varridos antes de pushes de checkpoint e branch. crates/core/src/git.rs:19-29,107-128; crates/core/src/engine.rs:703-740,1499-1531.
- Engine → GitHub/keyring; ASKPASS responde somente a prompts HTTPS de github.com. crates/core/src/credentials.rs:17-35,59-88; crates/core/src/signing.rs:13-73.

### Attacker Capabilities

- Colaborador/remoto hostil pode oferecer refs, commits, árvores e blobs; não se presume controle local, chave privada ou aprovação do dispositivo. crates/core/src/transport.rs:94-104; crates/core/src/engine.rs:787-908.
- Outro usuário local pode tentar acessar IPC e token; efetividade das ACLs Windows depende da instalação. crates/core/src/ipc.rs:82-110,130-136,167-199.
- Código na conta do operador compartilha autoridade local e pode usar --direct. crates/cli/src/main.rs:89-115.

### Security Objectives

- Checkpoint não modifica índice real, HEAD ou branch; aplicar exige checkpoint, plano seguro e reverificação. crates/core/src/snapshot.rs:54-79; crates/core/src/engine.rs:1058-1101; crates/core/src/apply.rs:485-495.
- Escritas de arquivos remotos permanecem confinadas ao root via handles relativos. crates/core/src/paths.rs:135-210.
- Pushes de checkpoint e branch varrem objetos novos; alertas bloqueantes impedem envio. crates/core/src/engine.rs:703-740,1499-1531.
- Aplicação automática exige projeto confiável, assinatura/chave/aprovação e plano elegível. crates/core/src/engine.rs:852-908,1033-1065,1609-1624.
- Credenciais/chave privada ficam no keyring; token GitHub via ASKPASS só para github.com. crates/core/src/credentials.rs:17-35,69-88; crates/core/src/signing.rs:13-35.

### Assumptions

- AUDITORIA_SEGURANCA.md é relatório anterior, dado não confiável. O código atual agora faz varredura de objetos alcançáveis, assina checkpoints e usa escrita confinada. crates/core/src/engine.rs:703-740,1499-1531; crates/core/src/snapshot.rs:107-126; crates/core/src/paths.rs:135-210.
- SECURITY.md afirma IPC por usuário; permissões explícitas no código são Unix, ACLs Windows ainda dependem do SO/biblioteca. crates/core/src/ipc.rs:82-110,130-136.
- CONTINUITY_DATA_DIR altera o destino dos dados e suas permissões são requisito de implantação. crates/core/src/lib.rs:27-42; crates/agent/src/main.rs:23-39.

## Findings

| Finding | Severity | Confidence | Detailed write-up |
| --- | --- | --- | --- |
| [Cliente Windows envia o token IPC a um pipe falso](#finding-1) | high | high | inline below |
| [Troca da chave do dispositivo não invalida checkpoints antigos](#finding-2) | medium | high | inline below |

### Confidence Scale

| Label | Meaning |
| --- | --- |
| high | Direct evidence supports the finding with no material unresolved blocker. |
| medium | Evidence supports a plausible issue, but material runtime or reachability proof remains. |
| low | Evidence is incomplete and the item is retained only for explicit follow-up. |

<a id="finding-1"></a>

### [1] Cliente Windows envia o token IPC a um pipe falso

| Field | Value |
| --- | --- |
| Severity | high |
| Confidence | high |
| Confidence rationale | Fluxo de nomeação, conexão, envio do token e aceitação do ping verificados no código e na dependência local interprocess 2.4.7. |
| Category | Falsificação de servidor IPC local |
| CWE | CWE-290 |
| Affected lines | crates/core/src/ipc.rs:59, crates/core/src/ipc.rs:235, apps/desktop/src-tauri/src/lib.rs:22, apps/desktop/src/pages/GitHubConnect.tsx:99 |

#### Summary

No Windows, o nome do named pipe é previsível e o cliente não autentica o servidor antes de enviar o token IPC e os parâmetros. Um usuário local pode reservar o pipe enquanto o Agent está parado, responder ao ping e capturar futuras chamadas da interface, incluindo um token GitHub informado manualmente.

#### Root Cause

O cliente autentica o servidor somente por alcançar o nome do pipe e receber uma resposta de ping; não verifica SID, processo ou outro segredo do servidor antes de transmitir o próprio token.

**Nome de pipe previsível** — `crates/core/src/ipc.rs:59`

Nome deriva do nome de usuário e hash do diretório de dados, sem segredo.

```rust
Endpoint { name: format!("continuity-agent-{}-{tag}", user_tag()), token_file }
```

**Cliente envia token sem validar servidor** — `crates/core/src/ipc.rs:235`

Conexão por nome não autentica a identidade do servidor antes do envio.

```rust
let conn = Stream::connect(self.endpoint.to_name()?)?;
```

**Token e parâmetros transmitidos** — `crates/core/src/ipc.rs:238`

A requisição contém o segredo do cliente e os parâmetros da operação.

```rust
let req = Request { id: 1, token: self.token.clone(), method: method.into(), params };
```

**Ping aceito como prova de Agent** — `apps/desktop/src-tauri/src/lib.rs:23`

Qualquer servidor que responda sucesso a ping é tratado como Agent.

```rust
if let Ok(c) = client()
        && c.is_alive()
    {
        return Ok(c);
```

#### Validation

Outro usuário local pode criar o pipe de nome previsível antes da inicialização do Agent. O desktop carrega agent.token e envia-o ao fazer o primeiro ping; se a resposta falsa indicar sucesso, chamadas posteriores continuam para o servidor controlado pelo invasor. O Agent legítimo pode falhar ao tentar vincular esse nome já ocupado.

Validation method: Revisão estática do fluxo e da dependência local

Counterevidence and remaining uncertainty:
- No servidor legítimo, o token é validado por requisição
- O arquivo do token é protegido pelas permissões do perfil do usuário, mas isso não autentica o servidor do cliente

#### Dataflow

Pipe previsível pré-reservado → desktop conecta e envia token no ping → ping falso aceito → chamadas futuras e parâmetros capturados.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

- **Attacker:** Outro usuário local capaz de criar um named pipe global com o nome calculável

- **Entry point:** \\.pipecontinuity-agent-\<USERNAME\>-\<hash-data-dir\>

- **Sink:** Token IPC e parâmetros de chamadas Tauri, incluindo github.set_token

Preconditions:
- Agent parado durante pre-bind
- Desktop iniciado enquanto pipe falso ocupa o nome

#### Severity

**High** — Outro usuário local pode interceptar o token IPC e dados de chamadas, podendo assumir a autoridade do Agent e obter credenciais enviadas pelo usuário.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Autenticar o servidor do named pipe no Windows antes de enviar qualquer requisição, verificando sua identidade/SID e restringindo ACLs do pipe ao usuário esperado. Não considerar um ping bem-sucedido prova suficiente da identidade do Agent.

<a id="finding-2"></a>

### [2] Troca da chave do dispositivo não invalida checkpoints antigos

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Reset altera apenas a linha de confiança do dispositivo; apply(auto) usa verified persistido na linha do checkpoint junto da aprovação atual do mesmo device ID. |
| Category | Estado de autenticação não invalidado após revogação de chave |
| CWE | CWE-345 |
| Affected lines | crates/core/src/db.rs:466, crates/core/src/engine.rs:886, crates/core/src/engine.rs:1033, crates/core/src/engine.rs:929 |

#### Summary

Ao esquecer a chave de um dispositivo, o banco revoga a aprovação e remove a chave fixada, mas mantém como verificados os checkpoints já importados. Depois que uma nova chave do mesmo ID é fixada e aprovada, checkpoints antigos assinados pela chave revogada ainda podem atender ao gate de aplicação automática.

#### Root Cause

A revogação remove a chave e aprovação do dispositivo, mas não invalida nem revalida a confiança autenticada anteriormente em cada checkpoint; a aplicação liga `row.verified` à aprovação atual do ID, não à chave que verificou a linha.

**Reset só limpa chave e aprovação** — `crates/core/src/db.rs:466`

A operação não atualiza checkpoints verificados do dispositivo.

```rust
self.conn.execute("UPDATE devices SET public_key = NULL, approved = 0 WHERE id = ?1", [id])?;
```

**Auto-apply confia em verified salvo** — `crates/core/src/engine.rs:1033`

O gate lê o booleano persistido na linha do checkpoint.

```rust
if !row.verified {
    return Ok(ApplyOutcome::NeedsManual { reason: "assinatura do checkpoint não verificada".into() });
}
```

**Aprovação consultada pelo ID atual** — `crates/core/src/engine.rs:1036`

A aprovação da nova chave é consultada pelo mesmo ID do dispositivo, sem revalidar qual chave assinou aquela linha antiga.

```rust
if !self.db().device_trust(&row.device_id)?.1 {
```

#### Validation

Se um checkpoint foi importado e marcado verified enquanto a chave antiga era fixada, reset_device_key deixa esse valor intacto. Uma nova chave pode ser pinada e aprovada para o mesmo ID; apply(auto) então aceita row.verified e a aprovação atual, embora não confira a assinatura antiga novamente contra a chave nova.

Validation method: Revisão estática de estado persistido e fluxo de aplicação

Counterevidence and remaining uncertainty:
- Reset revoga imediatamente a aprovação atual
- Novos checkpoints assinados por chave diferente não verificam contra a chave fixada após pinagem
- O cenário requer um checkpoint antigo ainda pendente e uma chave anterior comprometida/revogada

#### Dataflow

Checkpoint antigo importado/verificado → reset_device_key apaga somente chave/aprovação → nova chave é fixada e aprovada → incoming seleciona checkpoint pendente antigo → apply(auto) confia em verified e aprovação atual.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

- **Attacker:** Atacante com chave privada antiga comprometida ou checkpoints pré-existentes não confiáveis do dispositivo

- **Entry point:** Checkpoint assinado com a chave antiga e já importado

- **Sink:** Aplicação automática no worktree

Preconditions:
- Checkpoint antigo ainda não aplicado
- Reset e aprovação de nova chave para o mesmo device ID
- Projeto confiável e condições normais de auto-apply

#### Severity

**Medium** — Exploração exige checkpoint pré-existente assinado por chave comprometida/revogada e aprovação posterior da nova chave, mas pode reabilitar conteúdo antigo não confiável para escrita automática.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Ao resetar uma chave, marcar como não verificados todos os checkpoints daquele device ID; preferencialmente persistir a impressão digital da chave que verificou cada checkpoint e conferir igualdade com a chave atualmente fixada na aplicação automática.

## Reviewed Surfaces

| Surface | Risk Area | Outcome | Notes |
| --- | --- | --- | --- |
| Autenticação de IPC e identidade do Agent no Windows | Autoridade local e credenciais | Reported | Pipe global com nome previsível é aceito sem autenticação do servidor; o cliente envia token e parâmetros antes de validar peer. crates/core/src/ipc.rs:52-75,224-250; apps/desktop/src-tauri/src/lib.rs:17-39. |
| Sincronização, varredura de objetos e assinatura de checkpoints | Confidencialidade e integridade | No issue found | O estado atual varre objetos novos em checkpoints e branches, assina checkpoints e fixa chave por dispositivo; os achados anteriores não persistem no código atual. crates/core/src/engine.rs:703-740,1499-1531; crates/core/src/snapshot.rs:107-126; crates/core/src/signing.rs:40-59. |
| Escrita confinada e aplicação protegida | Integridade do sistema de arquivos | No issue found | Writes usam cap_std::fs::Dir e aplicam plano/checkpoint/hash; a corrida anterior de symlink foi corrigida. crates/core/src/paths.rs:135-210; crates/core/src/engine.rs:1058-1101. |
| UI, credenciais, CLI, Agent, testes e workflows | Dados e execução local | No issue found | Nenhum outro achado confirmado nesta revisão. A ACL Windows do pipe/token permanece central ao achado IPC. |

## Open Questions And Follow Up

- O SECURITY.md já documenta pipe squatting no Windows como risco conhecido e aponta verificação do SID do servidor como mitigação planejada.

## Status das correções

Atualizado após a remediação. Cada correção tem teste automatizado.

| Achado | Status | Correção | Teste |
| --- | --- | --- | --- |
| [1] Cliente Windows envia o token IPC a um pipe falso | ✅ Corrigido | Novo protocolo: o token **nunca** trafega no canal. O cliente abre a conexão com um nonce; o servidor responde com o próprio nonce e `HMAC-SHA256(token, "server" ‖ nonces)`; só se a prova conferir (comparação em tempo constante) o cliente envia a requisição, autenticada por `HMAC(token, "client" ‖ nonces)`. Um impostor não recebe token, método nem parâmetros, e o cliente retorna o erro `impostor` sem iniciar outro Agent por cima. Defesa em profundidade no Windows: pipe criado com DACL protegida `D:P(A;;GA;;;<SID do usuário>)` e como primeira instância (o Agent falha com mensagem clara se o nome estiver ocupado); o cliente recusa servidor cujo processo pertence a outra conta (`GetNamedPipeServerProcessId` + SID do token). `ping` aceito só depois do handshake. | `ipc::tests::client_sends_nothing_secret_to_a_squatter`, `ipc::tests::roundtrip_and_rejects_bad_token`, `ipc::tests::proofs_depend_on_token_role_and_both_nonces`, `ipc::winsec::tests::own_process_matches_and_descriptor_builds` |
| [2] Troca da chave do dispositivo não invalida checkpoints antigos | ✅ Corrigido | Duas barreiras independentes: (a) `reset_device_key` zera `verified` de todos os checkpoints daquele dispositivo na mesma transação; (b) a pré-visualização e a aplicação automática não confiam mais na coluna `verified`: relêem o checkpoint do Git e exigem assinatura válida **pela chave fixada no momento** e dispositivo aprovado. | `audit2_finding2_key_reset_invalidates_checkpoints_verified_with_the_old_key` |

Verificação:

- O teste do achado 2 foi executado contra o código anterior e **falhou**; com só uma das barreiras revertida ele também falha (cada barreira é necessária para o teste passar).
- O teste do achado 1 não roda contra o código anterior, porque o protocolo mudou. No código anterior, a primeira linha enviada pelo cliente era a própria requisição com o campo `token`.
- Verificado na instalação local: a ACL do pipe real (`Get-Acl \\.\pipe\continuity-agent-…`) mostra apenas o usuário atual com controle total; antes valia a DACL padrão (SYSTEM, Administradores, criador e leitura para Todos). A CLI conversa normalmente com o Agent pelo novo protocolo.
- Risco residual: um processo de outro usuário que ocupe o nome antes do Agent ainda causa negação de serviço (o Agent não inicia), mas não obtém dados. O app mostra a mensagem de canal não autenticado.

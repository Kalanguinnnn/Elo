# Política de segurança

## Relatar uma vulnerabilidade

Não abra issues públicas para vulnerabilidades. Use o recurso **Private vulnerability
reporting** do GitHub no repositório do projeto. A resposta inicial deve ocorrer em até
7 dias.

## Modelo de ameaças e proteções

| Risco | Proteção |
|---|---|
| Remoto hostil com caminhos `../`, `.git`, nomes reservados do Windows | `paths::validate_rel_path`; as entradas são ignoradas e reportadas no plano. |
| Escrita através de symlink plantado | `check_no_symlink_ancestors` (inclusive reparse points) como política, e todas as escritas/remoções via `cap_std::fs::Dir` aberto na raiz — mesmo uma troca concorrente de diretório por link/junção não leva para fora do projeto. Symlinks recebidos só são criados se o alvo permanece no projeto. |
| Checkpoint forjado por quem tem escrita no remoto | Assinatura Ed25519 por dispositivo (chave no cofre do SO), fixada no primeiro uso; aplicação automática só de dispositivos aprovados pelo usuário após comparar a impressão digital. Chave diferente para um dispositivo conhecido é registrada como possível falsificação. A assinatura é conferida de novo no momento da aplicação, contra a chave fixada naquele instante; redefinir a chave de um dispositivo invalida as verificações feitas com a chave anterior. |
| Sobrescrever trabalho local | Merge de três vias por arquivo, reverificação de hash antes de escrever, checkpoint de segurança e preservação da versão não escolhida. |
| Arquivo ignorado local substituído | Detectado como conflito `local_untracked`; ao substituir, é copiado antes para `.git/continuity/preserved`. |
| Force push / perda no remoto | Refs imutáveis; push apenas cria refs; a retenção só remove refs do próprio dispositivo. |
| Injeção de comandos | Git sempre via vetor de argumentos; URLs `ext::`/`fd::` e argumentos iniciados com `-` são rejeitados; variáveis `GIT_*` herdadas são removidas. |
| `.git/config` de projeto hostil | A configuração do próprio repositório não é confiável. Toda chamada do Git desliga `core.fsmonitor` e hooks e só permite os transportes file/http/https/ssh (`GIT_ALLOW_PROTOCOL`). A URL efetiva (após `insteadOf`) é validada antes de cada fetch/push. Se `.git/config`, `config.worktree` ou um arquivo incluído por eles define filtros, helpers de credencial, `core.askPass`, `uploadpack`/`receivepack` ou `remote.*.vcs`, o Elo não executa os comandos que os acionariam: pausa o projeto e mostra a chave. As mesmas chaves na configuração global/sistema, como o Git LFS, continuam valendo. |
| Vazamento de tokens | Cofre do SO (`keyring`); entrega via `GIT_ASKPASS` só para `github.com`; nunca em disco, argv ou URL. URLs de remoto com usuário/token são recusadas; as que já existiam no `.git/config` aparecem mascaradas (`https://***@host`) no status, no log e nas mensagens de erro do Git. |
| Envio de segredos | Varredura de **todos os objetos** que o push publicaria (worktree, staging, commits locais), inclusive no push de branch do modo de commits automáticos; sem limites silenciosos (arquivos > 50 MB exigem decisão). Achados bloqueantes ficam em quarentena local; alertas exigem confirmação individual; o segredo não é armazenado (apenas um trecho mascarado e uma impressão digital). |
| Outro usuário local lendo o perfil | Pasta de dados `0700` e banco `0600` (Unix) aplicados ao abrir o Engine, em todos os pontos de entrada, inclusive `continuity --direct`. No Windows o perfil fica em `%LOCALAPPDATA%`, com ACL do usuário. |
| Outro usuário local controlando o Agent | Named pipe/socket por usuário + token aleatório em arquivo legível apenas pelo usuário (no Windows, ACL herdada do perfil: SYSTEM, Administradores e o usuário — verificado com `icacls`). O token nunca trafega no canal: cada conexão começa com troca de nonces em que o **servidor prova primeiro** conhecer o token (HMAC-SHA256) e só então o cliente envia a requisição, com a própria prova. No Windows o pipe é criado com DACL protegida só para o usuário atual, como primeira instância, e o cliente recusa um servidor de outra conta. |
| Aplicação automática indevida | Desativada por padrão; requer projeto confiável, dispositivo aprovado com assinatura válida, avanço direto, mesma branch, nenhum editor conhecido em execução e nenhuma edição recente. |

## Fora do escopo atual

- Builds não são assinadas digitalmente (requisito do projeto: sem assinatura, DRM ou licenciamento comercial).
- A detecção de editores é heurística (lista de processos). Na dúvida, o sistema sempre exige ação manual.
- *Pipe squatting* no Windows: um processo de outro usuário que crie o named pipe antes do Agent não recebe mais nada (o cliente o identifica como impostor), mas ainda impede o Agent de iniciar até ser encerrado (negação de serviço local).
- Se o cofre de credenciais de um computador for apagado, ele gera uma nova chave e os outros passam a acusar "chave diferente": é preciso reaprovar o dispositivo.
- Releases: actions de terceiros fixadas por SHA completo e escrita só no job de release (verificado por `crates/core/tests/workflow_policy.rs`); ao atualizar uma action, revise o diff da nova revisão antes de trocar o SHA.
- Auditorias e status das correções: [AUDITORIA_SEGURANCA.md](AUDITORIA_SEGURANCA.md), [AUDITORIA_SEGURANCA_2.md](AUDITORIA_SEGURANCA_2.md) e [AUDITORIA_SEGURANCA_3.md](AUDITORIA_SEGURANCA_3.md).

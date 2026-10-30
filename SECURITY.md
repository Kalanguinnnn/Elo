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
| Checkpoint forjado por quem tem escrita no remoto | Assinatura Ed25519 por dispositivo (chave no cofre do SO), fixada no primeiro uso; aplicação automática só de dispositivos aprovados pelo usuário após comparar a impressão digital. Chave diferente para um dispositivo conhecido é registrada como possível falsificação. |
| Sobrescrever trabalho local | Merge de três vias por arquivo, reverificação de hash antes de escrever, checkpoint de segurança e preservação da versão não escolhida. |
| Arquivo ignorado local substituído | Detectado como conflito `local_untracked`; ao substituir, é copiado antes para `.git/continuity/preserved`. |
| Force push / perda no remoto | Refs imutáveis; push apenas cria refs; a retenção só remove refs do próprio dispositivo. |
| Injeção de comandos | Git sempre via vetor de argumentos; URLs `ext::`/`fd::` e argumentos iniciados com `-` são rejeitados; variáveis `GIT_*` herdadas são removidas. |
| Vazamento de tokens | Cofre do SO (`keyring`); entrega via `GIT_ASKPASS` só para `github.com`; nunca em disco, argv ou URL. |
| Envio de segredos | Varredura de **todos os objetos** que o push publicaria (worktree, staging, commits locais), inclusive no push de branch do modo de commits automáticos; sem limites silenciosos (arquivos > 50 MB exigem decisão). Achados bloqueantes ficam em quarentena local; alertas exigem confirmação individual; o segredo não é armazenado (apenas um trecho mascarado e uma impressão digital). |
| Outro usuário local controlando o Agent | Named pipe/socket por usuário + token aleatório em arquivo legível apenas pelo usuário (no Windows, ACL herdada do perfil: SYSTEM, Administradores e o usuário — verificado com `icacls`). |
| Aplicação automática indevida | Desativada por padrão; requer projeto confiável, dispositivo aprovado com assinatura válida, avanço direto, mesma branch, nenhum editor conhecido em execução e nenhuma edição recente. |

## Fora do escopo atual

- Builds não são assinadas digitalmente (requisito do projeto: sem assinatura, DRM ou licenciamento comercial).
- A detecção de editores é heurística (lista de processos). Na dúvida, o sistema sempre exige ação manual.
- *Pipe squatting* no Windows: um processo de outro usuário que crie o named pipe antes do Agent poderia receber o token do cliente (renovado a cada início do Agent). Mitigação planejada: verificar o SID do servidor antes de enviar o token.
- Se o cofre de credenciais de um computador for apagado, ele gera uma nova chave e os outros passam a acusar "chave diferente": é preciso reaprovar o dispositivo.
- Auditoria completa e status das correções: [AUDITORIA_SEGURANCA.md](AUDITORIA_SEGURANCA.md).

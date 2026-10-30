# Política de segurança

## Relatar uma vulnerabilidade

Não abra issues públicas para vulnerabilidades. Use o recurso **Private vulnerability
reporting** do GitHub no repositório do projeto. A resposta inicial deve ocorrer em até
7 dias.

## Modelo de ameaças e proteções

| Risco | Proteção |
|---|---|
| Remoto hostil com caminhos `../`, `.git`, nomes reservados do Windows | `paths::validate_rel_path`; as entradas são ignoradas e reportadas no plano. |
| Escrita através de symlink plantado | `check_no_symlink_ancestors` (inclusive reparse points); symlinks recebidos só são criados se o alvo permanece no projeto. |
| Sobrescrever trabalho local | Merge de três vias por arquivo, reverificação de hash antes de escrever, checkpoint de segurança e preservação da versão não escolhida. |
| Arquivo ignorado local substituído | Detectado como conflito `local_untracked`; ao substituir, é copiado antes para `.git/continuity/preserved`. |
| Force push / perda no remoto | Refs imutáveis; push apenas cria refs; a retenção só remove refs do próprio dispositivo. |
| Injeção de comandos | Git sempre via vetor de argumentos; URLs `ext::`/`fd::` e argumentos iniciados com `-` são rejeitados; variáveis `GIT_*` herdadas são removidas. |
| Vazamento de tokens | Cofre do SO (`keyring`); entrega via `GIT_ASKPASS` só para `github.com`; nunca em disco, argv ou URL. |
| Envio de segredos | Varredura antes de cada push; achados bloqueantes ficam em quarentena local; alertas exigem confirmação individual; o segredo não é armazenado (apenas um trecho mascarado e uma impressão digital). |
| Outro usuário local controlando o Agent | Named pipe/socket por usuário + token aleatório em arquivo legível apenas pelo usuário. |
| Aplicação automática indevida | Desativada por padrão; requer projeto confiável, avanço direto, mesma branch, nenhum editor conhecido em execução e nenhuma edição recente. |

## Fora do escopo atual

- Builds não são assinadas digitalmente (requisito do projeto: sem assinatura, DRM ou licenciamento comercial).
- A detecção de editores é heurística (lista de processos). Na dúvida, o sistema sempre exige ação manual.
- Commits intermediários não enviados podem conter segredos já removidos do estado final: a varredura cobre o estado publicado, não cada commit.

# Roadmap

## Beta (atual)
- [x] M1 — Fundação e núcleo: checkpoints isolados, transporte entre clones, recuperação, identidade de dispositivo.
- [x] M2 — Automação e app: Agent, monitoramento híbrido, fila offline, aplicação protegida, confiança, conflitos, histórico, retenção, segredos, modo de commits automáticos, GitHub e UI.
- [ ] M3 — Estabilização e distribuição: CI verde em Windows/Linux no GitHub, instaladores Linux gerados no CI, primeira release em rascunho.

## Próximos
- Registrar um OAuth App do GitHub para habilitar o login pelo navegador por padrão.
- Leitura em lote (`git cat-file --batch --filters`) para acelerar a aplicação e a restauração de muitos arquivos.
- Ícone na bandeja do sistema e notificações nativas.
- Suporte a submódulos (hoje são ignorados e reportados).
- macOS (fora do escopo inicial).

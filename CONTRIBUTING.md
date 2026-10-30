# Como contribuir

1. Leia [AGENTS.md](AGENTS.md): ele contém as invariantes que nenhuma mudança pode quebrar.
2. Ambiente: Rust estável (1.89+), Node 22+, Git 2.31+. No Linux, instale as
   dependências do Tauri listadas em [docs/INSTALL.md](docs/INSTALL.md).
3. Antes de abrir um PR:
   ```bash
   cargo clippy --all-targets -- -D warnings
   cargo test
   cd apps/desktop && npm test && npm run build
   ```
4. Mudanças de comportamento de sincronização exigem testes de integração com repositórios
   temporários (`crates/core/tests`). Nunca use remotos reais nos testes.
5. Atualize `docs/IMPLEMENTATION_STATUS.md` quando um recurso mudar de estado.
6. Mensagens de commit no formato *Conventional Commits* (`feat(core): …`, `fix(agent): …`).

Relatos de segurança: siga o [SECURITY.md](SECURITY.md), não abra issues públicas.

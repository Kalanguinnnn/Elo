# Elo

Continue seu trabalho em outro computador — **inclusive alterações não commitadas** — sem
executar comandos Git. O Elo cria *checkpoints* do diretório de trabalho em
referências próprias do **seu** remoto Git e os aplica com segurança no outro computador.

- Não altera staging, HEAD ou branch ao criar checkpoints.
- Nunca usa force push; refs de checkpoint são imutáveis.
- Funciona offline: envios ficam numa fila persistente com backoff.
- Aplicação automática apenas em projetos confiáveis e quando é comprovadamente segura;
  caso contrário, pede ação manual. Conflitos são resolvidos lado a lado, sem perder a
  versão não escolhida.
- Verifica possíveis segredos antes de cada envio.
- Nada é enviado a servidores do Elo (não existem servidores do Elo).

Plataformas: **Windows** e **Linux**. Stack: Tauri 2, React + TypeScript + Vite, Rust,
Git CLI e SQLite.

## Como funciona

```
Computador A: edição → checkpoint (índice Git isolado) → push refs/continuity/checkpoints/<dispositivo>/<id>
Computador B: fetch → pré-visualização → checkpoint de segurança → merge de três vias por arquivo → arquivos, branch e staging
```

Os detalhes estão em [ARCHITECTURE.md](ARCHITECTURE.md).

## Instalação

Veja [docs/INSTALL.md](docs/INSTALL.md). Requisito: **Git 2.31+** no `PATH`.

## Uso rápido

1. Abra o aplicativo, dê um nome a este computador e (opcionalmente) conecte o GitHub.
2. Adicione um projeto: uma pasta local (com ou sem Git) ou um clone de um remoto.
3. Trabalhe normalmente. Checkpoints são criados a cada 10 minutos (configurável) e
   quando você usa **Sincronizar e encerrar**.
4. No outro computador, adicione o mesmo projeto (clone) e aplique a atualização
   recebida em **Projetos → Atualizações de outros computadores**.

CLI equivalente (`continuity --help`; o binário mantém o nome interno do projeto):

```bash
continuity add ~/code/meu-projeto
continuity checkpoint meu-projeto
continuity sync meu-projeto
continuity incoming meu-projeto
continuity apply meu-projeto <checkpoint-id>
```

## Desenvolvimento

```bash
cargo test                                   # motor, agent e CLI (repos Git temporários)
cd apps/desktop && npm ci && npm test        # UI
node scripts/prepare-sidecars.mjs --debug    # binários do agent/CLI para o Tauri
npx tauri dev                                # app desktop
```

Antes de contribuir, leia [AGENTS.md](AGENTS.md), [CONTRIBUTING.md](CONTRIBUTING.md) e
[SECURITY.md](SECURITY.md). O estado de cada recurso está em
[docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md).

## Licença

Gratuito e de código aberto, sob a [licença MIT](LICENSE).

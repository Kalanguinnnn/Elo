# Instalação

Requisito em todas as plataformas: **Git 2.31 ou superior** disponível no `PATH`.

## Windows

1. Baixe `Continuity_<versão>_x64-setup.exe` (NSIS) ou `Continuity_<versão>_x64_en-US.msi`
   na página de *Releases*.
2. As builds beta **não são assinadas**, então o SmartScreen pode exibir um aviso. Use
   "Mais informações → Executar assim mesmo" somente se o arquivo veio da página oficial
   de releases.
3. O instalador inclui o app, o Agent (`continuity-agent.exe`) e a CLI (`continuity.exe`).
   O WebView2 já vem no Windows 10/11; se faltar, o instalador o obtém.

Os dados ficam em `%LOCALAPPDATA%\Continuity`. O token do GitHub fica no Gerenciador de
Credenciais do Windows.

## Linux

- `.deb` (Debian/Ubuntu): `sudo apt install ./continuity_<versão>_amd64.deb`
- `.rpm` (Fedora/openSUSE): `sudo dnf install ./continuity-<versão>.x86_64.rpm`
- AppImage: `chmod +x Continuity_<versão>_amd64.AppImage && ./Continuity_<versão>_amd64.AppImage`

Os dados ficam em `~/.local/share/continuity`. O token do GitHub fica no Secret Service
(GNOME Keyring/KWallet), que precisa estar ativo na sessão.

## Compilar a partir do código

```bash
# Linux: dependências do Tauri
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libdbus-1-dev libssl-dev libxdo-dev build-essential pkg-config patchelf

cd apps/desktop
npm ci
node scripts/prepare-sidecars.mjs        # compila o agent e a CLI em release
npx tauri build                          # gera os instaladores em target/release/bundle
```

No Windows são necessários o Visual Studio Build Tools (C++) e o WebView2.

## Iniciar com o sistema

Em **Configurações → Agent em segundo plano**, ative "Iniciar o Agent ao entrar no
sistema". No Windows isso cria a chave `HKCU\…\Run\ContinuityAgent`; no Linux,
`~/.config/autostart/continuity-agent.desktop`.

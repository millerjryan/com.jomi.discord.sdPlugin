# Discord Client Controls

This repository contains a native Rust OpenDeck or Stream Deck style plugin that controls the local Discord desktop client through Discord RPC.

The plugin includes three actions:
- **Voice Channel**: Join a configured voice or stage channel.
- **Screenshare**: Toggle Discord screenshare.
- **Soundboard**: Play a configured server soundboard sound.

All actions share one global OAuth configuration (`clientId`, `clientSecret`, `redirectUri`, `accessToken`) so authorization only needs to happen once.

## What the plugin does

- Stores Discord OAuth credentials and token as global plugin settings.
- Reuses global authorization across all Discord actions.
- Loads Discord servers from the local desktop client session.
- For Voice Channel: loads channels for a selected server and joins the selected channel on key press.
- For Screenshare: toggles Discord screenshare on key press.
- For Soundboard: loads soundboard sounds, separates by server, then lets you select one sound per button.

## Requirements

- Discord desktop client running on the same machine.
- A Discord application with RPC enabled.
- The application `clientId`, `clientSecret`, and redirect URI from the Discord developer portal.
- The user must approve the OAuth prompt the first time the property inspector connects.

## OAuth scopes used

- `rpc`
- `identify`
- `guilds`
- `rpc.voice.write`
- `rpc.screenshare.write`

## Action setup

1. Add any Discord action to a key.
2. Enter and save credentials in the property inspector.
3. Click Connect Discord and approve authorization.
4. Configure action-specific settings:
- Voice Channel: pick server, then channel.
- Soundboard: pick server, then sound.
- Screenshare: no per-button target selection required.

## Build

### Linux

```bash
./scripts/build-linux.sh
```

### Windows

```powershell
./scripts/build-windows.ps1
```

### CI artifacts

GitHub Actions builds Linux and Windows binaries from `.github/workflows/build.yml`.

## Package layout

- `manifest.json`: plugin manifest with Windows and Linux code paths.
- `native`: Rust source for the executable plugin runtime.
- `ui`: property inspector HTML and JavaScript.
- `imgs`: extensionless SVG assets used by the manifest.

## Notes

- The plugin uses Discord RPC commands (`GET_GUILDS`, `GET_CHANNELS`, `GET_SOUNDBOARD_SOUNDS`, `SELECT_VOICE_CHANNEL`, `TOGGLE_SCREENSHARE`, `PLAY_SOUNDBOARD_SOUND`) and controls the local desktop client, not Discord's public REST API.
- Voice channel selection retries with `force: true` only when Discord returns `5003`.
- Soundboard lists are grouped by server in the property inspector and stored per button (`guildId` + `soundId`).
- The Soundboard property inspector includes an in-UI debug panel for payload/filter troubleshooting.
- If your Discord application requires additional OAuth configuration for code exchange, update it in the developer portal before connecting.

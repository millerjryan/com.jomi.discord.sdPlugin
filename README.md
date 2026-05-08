# Discord Client Controls

This repository contains a native Rust OpenDeck or Stream Deck style plugin with one action: **Voice Channel**.

Pressing the action asks the local Discord desktop client to join the configured voice channel through Discord's local RPC interface. The property inspector stores a shared `clientId` and `clientSecret` as global plugin settings so later Discord actions can reuse the same OAuth configuration.

## What the plugin does

- Stores Discord `clientId` and `clientSecret` globally for the plugin.
- Authorizes against the running Discord desktop client with OAuth scopes needed for RPC and guild discovery.
- Populates the property inspector with Discord servers from the current client session.
- Populates the selected server's voice or stage channels.
- Joins the selected voice channel when the button is pressed.

## Requirements

- Discord desktop client running on the same machine.
- A Discord application with RPC enabled.
- The application `clientId` and `clientSecret` from the Discord developer portal.
- The user must approve the OAuth prompt the first time the property inspector connects.

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

- The plugin uses Discord RPC `GET_GUILDS`, `GET_CHANNELS`, and `SELECT_VOICE_CHANNEL` commands, so it controls the local client rather than Discord's public REST API.
- The action uses `force: true` for voice selection so pressing the button moves the current user into the chosen voice channel.
- If your Discord application requires additional OAuth configuration for code exchange, update it in the developer portal before connecting.

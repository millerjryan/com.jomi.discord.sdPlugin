#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$HOME/.cargo/env"

cargo build --manifest-path "$ROOT/native/Cargo.toml" --release
mkdir -p "$ROOT/bin"
cp "$ROOT/native/target/release/discord_client_controls" "$ROOT/bin/discord-client-controls"
chmod +x "$ROOT/bin/discord-client-controls"

echo "Linux binary copied to $ROOT/bin/discord-client-controls"

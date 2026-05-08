$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'

cargo build --manifest-path "$root/native/Cargo.toml" --release
New-Item -ItemType Directory -Force -Path "$root/bin" | Out-Null
Copy-Item "$root/native/target/release/discord_client_controls.exe" "$root/bin/discord-client-controls.exe" -Force
Write-Host "Windows binary copied to $root/bin/discord-client-controls.exe"

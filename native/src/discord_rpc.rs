use std::env;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use uuid::Uuid;

use crate::models::{ChannelSummary, GuildSummary, SoundboardSoundSummary};

const HANDSHAKE: u32 = 0;
const FRAME: u32 = 1;
const CLOSE: u32 = 2;
const PING: u32 = 3;
const PONG: u32 = 4;
const OAUTH_SCOPES: [&str; 5] = [
    "rpc",
    "identify",
    "guilds",
    "rpc.voice.write",
    "rpc.screenshare.write",
];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

pub struct DiscordRpcClient {
    connected: bool,
    authenticated: bool,
    client_id: String,
    access_token: Option<String>,
    granted_scopes: Vec<String>,
    pending_requests: HashMap<String, String>,
    transport: DiscordTransport,
}

impl DiscordRpcClient {
    pub async fn connect_with_token(client_id: &str, access_token: &str) -> Result<Self> {
        let mut client = Self::connect(client_id).await?;
        let scopes = client.authenticate(access_token).await?;
        if !scopes.iter().any(|scope| scope == "rpc.screenshare.write") {
            bail!(
                "Discord authorization did not grant the rpc.screenshare.write scope required for screenshare toggling"
            );
        }
        client.authenticated = true;
        client.access_token = Some(access_token.to_owned());
        client.granted_scopes = scopes;
        Ok(client)
    }

    pub async fn connect_authorized(
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        cached_access_token: Option<&str>,
    ) -> Result<(Self, String)> {
        if let Some(access_token) = cached_access_token {
            let mut client = Self::connect(client_id).await?;
            if let Ok(scopes) = client.authenticate(access_token).await {
                if scopes.iter().any(|scope| scope == "rpc.voice.write") {
                    client.authenticated = true;
                    client.access_token = Some(access_token.to_owned());
                    client.granted_scopes = scopes;
                    return Ok((client, access_token.to_owned()));
                }
            }
        }

        let mut client = Self::connect(client_id).await?;
        let code = client.authorize(client_id).await.map_err(|error| {
            anyhow!(
                "Discord authorization failed. Make sure your Discord app includes your user in the RPC tester allowlist and only requests approved scopes (rpc, identify, guilds). Original error: {error}"
            )
        })?;
        let access_token = exchange_code(client_id, client_secret, redirect_uri, &code).await?;
        let scopes = client.authenticate(&access_token).await?;
        if !scopes.iter().any(|scope| scope == "rpc.voice.write") {
            bail!("Discord authorization did not grant the rpc.voice.write scope required to join voice channels");
        }
        client.authenticated = true;
        client.access_token = Some(access_token.clone());
        client.granted_scopes = scopes;
        Ok((client, access_token))
    }

    pub async fn get_guilds(&mut self) -> Result<Vec<GuildSummary>> {
        #[derive(Deserialize)]
        struct GuildsResponse {
            guilds: Vec<GuildSummary>,
        }

        let response = self.send_command("GET_GUILDS", json!({})).await?;
        let mut payload = serde_json::from_value::<GuildsResponse>(response["data"].clone())
            .context("invalid guild list response")?;
        for guild in &mut payload.guilds {
            guild.icon_url = guild.normalized_icon_url();
        }
        Ok(payload.guilds)
    }

    pub async fn get_channels(&mut self, guild_id: &str) -> Result<Vec<ChannelSummary>> {
        #[derive(Deserialize)]
        struct ChannelsResponse {
            channels: Vec<ChannelSummary>,
        }

        let response = self
            .send_command("GET_CHANNELS", json!({ "guild_id": guild_id }))
            .await?;
        let payload = serde_json::from_value::<ChannelsResponse>(response["data"].clone())
            .context("invalid channel list response")?;
        Ok(payload
            .channels
            .into_iter()
            .filter(ChannelSummary::is_voice_like)
            .collect())
    }

    pub async fn get_soundboard_sounds(&mut self, guild_id: &str) -> Result<Vec<SoundboardSoundSummary>> {
        let response = self
            .send_command("GET_SOUNDBOARD_SOUNDS", json!({ "guild_id": guild_id }))
            .await?;

        let data = response.get("data").cloned().unwrap_or_else(|| json!({}));
        let sounds = if let Some(array) = data.as_array() {
            array.clone()
        } else {
            data.get("sounds")
                .or_else(|| data.get("soundboard_sounds"))
                .or_else(|| data.get("items"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };

        Ok(sounds
            .into_iter()
            .filter_map(|raw| {
                let sound_object = raw.get("sound").and_then(Value::as_object);
                let guild_object = raw.get("guild").and_then(Value::as_object);
                let sound_id = raw
                    .get("sound_id")
                    .or_else(|| raw.get("id"))
                    .or_else(|| raw.get("soundId"))
                    .or_else(|| sound_object.and_then(|obj| obj.get("sound_id")))
                    .or_else(|| sound_object.and_then(|obj| obj.get("id")))
                    .map(value_to_string)
                    .unwrap_or_default();
                if sound_id.is_empty() {
                    return None;
                }

                let parsed_guild_id = raw
                    .get("guild_id")
                    .or_else(|| raw.get("guildId"))
                    .or_else(|| guild_object.and_then(|obj| obj.get("id")))
                    .map(value_to_string)
                    .unwrap_or_default();
                let resolved_guild_id = if parsed_guild_id.trim().is_empty() {
                    guild_id.to_owned()
                } else {
                    parsed_guild_id
                };

                let sound_name = raw
                    .get("name")
                    .or_else(|| raw.get("sound_name"))
                    .or_else(|| raw.get("soundName"))
                    .or_else(|| sound_object.and_then(|obj| obj.get("name")))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();

                Some(SoundboardSoundSummary {
                    guild_id: resolved_guild_id,
                    guild_name: String::new(),
                    sound_id,
                    sound_name,
                })
            })
            .collect())
    }

    pub async fn select_voice_channel(&mut self, channel_id: &str) -> Result<()> {
        let normal_join = self
            .send_command(
                "SELECT_VOICE_CHANNEL",
                json!({
                    "channel_id": channel_id,
                    "navigate": true,
                    "timeout": 30,
                }),
            )
            .await;

        match normal_join {
            Ok(_) => Ok(()),
            Err(error) => {
                // Discord requires `force` only when it returns error 5003.
                if !error.to_string().contains("Discord RPC error 5003") {
                    return Err(error);
                }

                self.send_command(
                    "SELECT_VOICE_CHANNEL",
                    json!({
                        "channel_id": channel_id,
                        "force": true,
                        "navigate": true,
                        "timeout": 30,
                    }),
                )
                .await?;
                Ok(())
            }
        }
    }

    pub async fn disconnect_voice_channel(&mut self) -> Result<()> {
        self.send_command(
            "SELECT_VOICE_CHANNEL",
            json!({
                "channel_id": Value::Null,
                "navigate": false,
                "timeout": 30,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn toggle_screenshare(&mut self) -> Result<()> {
        self.reconnect_if_needed().await?;
        self.send_command("TOGGLE_SCREENSHARE", json!({})).await?;
        Ok(())
    }

    pub async fn play_soundboard_sound(&mut self, guild_id: &str, sound_id: &str) -> Result<()> {
        self.reconnect_if_needed().await?;
        self.send_command(
            "PLAY_SOUNDBOARD_SOUND",
            json!({
                "guild_id": guild_id,
                "sound_id": sound_id,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn reconnect_if_needed(&mut self) -> Result<()> {
        if self.connected {
            return Ok(());
        }

        let mut refreshed = Self::connect(&self.client_id).await?;
        if let Some(token) = self.access_token.clone() {
            let scopes = refreshed.authenticate(&token).await?;
            refreshed.authenticated = true;
            refreshed.access_token = Some(token);
            refreshed.granted_scopes = scopes;
        }

        *self = refreshed;
        Ok(())
    }

    pub async fn close(&mut self) -> Result<()> {
        let _ = write_frame(&mut self.transport, CLOSE, &json!({})).await;
        self.transport.shutdown().await?;
        self.connected = false;
        self.authenticated = false;
        self.pending_requests.clear();
        Ok(())
    }

    async fn connect(client_id: &str) -> Result<Self> {
        let mut transport = DiscordTransport::connect().await?;
        let handshake = json!({
            "v": 1,
            "client_id": client_id,
        });
        write_frame(&mut transport, HANDSHAKE, &handshake).await?;

        loop {
            let (opcode, payload) = read_frame(&mut transport).await?;
            match opcode {
                FRAME => {
                    if payload.get("evt").and_then(Value::as_str) == Some("READY") {
                        return Ok(Self {
                            connected: true,
                            authenticated: false,
                            client_id: client_id.to_owned(),
                            access_token: None,
                            granted_scopes: Vec::new(),
                            pending_requests: HashMap::new(),
                            transport,
                        });
                    }
                }
                PING => write_frame(&mut transport, PONG, &json!({})).await?,
                CLOSE => bail!("Discord RPC socket closed during handshake"),
                _ => {}
            }
        }
    }

    async fn authorize(&mut self, client_id: &str) -> Result<String> {
        let response = self
            .send_command(
                "AUTHORIZE",
                json!({
                    "client_id": client_id,
                    "scopes": OAUTH_SCOPES,
                }),
            )
            .await?;

        response
            .get("data")
            .and_then(|data| data.get("code"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("Discord did not return an authorization code"))
    }

    async fn authenticate(&mut self, access_token: &str) -> Result<Vec<String>> {
        let response = self
            .send_command(
                "AUTHENTICATE",
                json!({
                    "access_token": access_token,
                }),
            )
            .await?;

        let scopes = response
            .get("data")
            .and_then(|data| data.get("scopes"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<String>>()
            })
            .ok_or_else(|| anyhow!("Discord did not return granted scopes during authentication"))?;

        Ok(scopes)
    }

    async fn send_command(&mut self, command: &str, args: Value) -> Result<Value> {
        let nonce = Uuid::new_v4().to_string();
        self.pending_requests
            .insert(nonce.clone(), command.to_owned());

        let command_result = timeout(REQUEST_TIMEOUT, async {
            write_frame(
                &mut self.transport,
                FRAME,
                &json!({
                    "cmd": command,
                    "args": args,
                    "nonce": nonce,
                }),
            )
            .await?;

            loop {
                let (opcode, payload) = read_frame(&mut self.transport).await?;
                match opcode {
                    FRAME => {
                        if payload.get("nonce").and_then(Value::as_str) != Some(nonce.as_str()) {
                            continue;
                        }
                        if payload.get("cmd").and_then(Value::as_str) == Some("ERROR")
                            || payload.get("evt").and_then(Value::as_str) == Some("ERROR")
                        {
                            let message = payload
                                .get("data")
                                .and_then(|data| data.get("message"))
                                .and_then(Value::as_str)
                                .unwrap_or("Discord RPC error");
                            let code = payload
                                .get("data")
                                .and_then(|data| data.get("code"))
                                .and_then(Value::as_i64)
                                .unwrap_or_default();
                            bail!("Discord RPC error {code}: {message}");
                        }
                        return Ok(payload);
                    }
                    PING => write_frame(&mut self.transport, PONG, &json!({})).await?,
                    CLOSE => bail!("Discord RPC socket closed while waiting for {command}"),
                    _ => {}
                }
            }
        })
        .await;

        self.pending_requests.remove(&nonce);

        match command_result {
            Ok(result) => result,
            Err(_) => bail!("Discord RPC request timed out for command {command}"),
        }
    }
}

#[derive(Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
}

async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code: &str,
) -> Result<String> {
    let client = Client::builder()
        .user_agent("discord-opendeck-plugin/0.1.0")
        .build()
        .context("failed to build OAuth client")?;

    let response = client
        .post("https://discord.com/api/oauth2/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
            ("code", code),
        ])
        .send()
        .await
        .context("failed to exchange Discord authorization code")?
        .error_for_status()
        .context("Discord rejected the authorization code exchange")?;

    let token = response
        .json::<OAuthTokenResponse>()
        .await
        .context("failed to decode OAuth token response")?;
    Ok(token.access_token)
}

enum DiscordTransport {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    Pipe(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl DiscordTransport {
    async fn connect() -> Result<Self> {
        #[cfg(unix)]
        {
            let stream = connect_unix().await?;
            return Ok(Self::Unix(stream));
        }

        #[cfg(windows)]
        {
            let pipe = connect_windows().await?;
            return Ok(Self::Pipe(pipe));
        }

        #[cfg(not(any(unix, windows)))]
        bail!("unsupported platform")
    }

    async fn shutdown(&mut self) -> Result<()> {
        match self {
            #[cfg(unix)]
            DiscordTransport::Unix(stream) => stream
                .shutdown()
                .await
                .context("failed to shutdown Discord IPC stream"),
            #[cfg(windows)]
            DiscordTransport::Pipe(pipe) => pipe
                .flush()
                .await
                .context("failed to flush Discord IPC pipe"),
        }
    }
}

#[cfg(unix)]
async fn connect_unix() -> Result<tokio::net::UnixStream> {
    let mut roots = Vec::<PathBuf>::new();
    if let Some(path) = env::var_os("XDG_RUNTIME_DIR") {
        roots.push(PathBuf::from(path));
    }
    if let Some(path) = env::var_os("TMPDIR") {
        roots.push(PathBuf::from(path));
    }
    roots.push(PathBuf::from("/tmp"));
    roots.push(PathBuf::from("/var/tmp"));

    for root in roots {
        for index in 0..10 {
            let candidate = root.join(format!("discord-ipc-{index}"));
            if !candidate.exists() {
                continue;
            }
            if let Ok(stream) = tokio::net::UnixStream::connect(&candidate).await {
                return Ok(stream);
            }
        }
    }

    bail!("could not find a running Discord IPC socket")
}

#[cfg(windows)]
async fn connect_windows() -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;

    for index in 0..10 {
        let candidate = format!(r"\\.\pipe\discord-ipc-{index}");
        if let Ok(pipe) = ClientOptions::new().open(&candidate) {
            return Ok(pipe);
        }
    }

    bail!("could not find a running Discord IPC pipe")
}

async fn write_frame(transport: &mut DiscordTransport, opcode: u32, payload: &Value) -> Result<()> {
    let body = serde_json::to_vec(payload).context("failed to serialize Discord RPC payload")?;
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    write_all(transport, &frame).await
}

async fn read_frame(transport: &mut DiscordTransport) -> Result<(u32, Value)> {
    let mut header = [0_u8; 8];
    read_exact(transport, &mut header).await?;
    let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let length = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut body = vec![0_u8; length];
    read_exact(transport, &mut body).await?;
    let payload = serde_json::from_slice::<Value>(&body).context("invalid Discord RPC JSON")?;
    Ok((opcode, payload))
}

async fn write_all(transport: &mut DiscordTransport, bytes: &[u8]) -> Result<()> {
    match transport {
        #[cfg(unix)]
        DiscordTransport::Unix(stream) => stream.write_all(bytes).await.context("failed to write Discord IPC payload"),
        #[cfg(windows)]
        DiscordTransport::Pipe(pipe) => pipe.write_all(bytes).await.context("failed to write Discord IPC payload"),
    }
}

async fn read_exact(transport: &mut DiscordTransport, bytes: &mut [u8]) -> Result<()> {
    match transport {
        #[cfg(unix)]
        DiscordTransport::Unix(stream) => stream.read_exact(bytes).await.context("failed to read Discord IPC payload").map(|_| ()),
        #[cfg(windows)]
        DiscordTransport::Pipe(pipe) => pipe.read_exact(bytes).await.context("failed to read Discord IPC payload").map(|_| ()),
    }
}

fn value_to_string(value: &Value) -> String {
    if let Some(as_str) = value.as_str() {
        return as_str.to_owned();
    }

    if let Some(as_u64) = value.as_u64() {
        return as_u64.to_string();
    }

    if let Some(as_i64) = value.as_i64() {
        return as_i64.to_string();
    }

    String::new()
}

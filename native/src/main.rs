mod discord_rpc;
mod models;
mod stream_deck;

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use models::{ChannelSummary, GlobalSettings, GuildSummary, VoiceChannelSettings};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::discord_rpc::DiscordRpcClient;
use crate::stream_deck::StreamDeckClient;

#[derive(Clone, Default)]
struct ActionState {
    action_uuid: String,
    settings: VoiceChannelSettings,
    connected: bool,
}

#[derive(Clone)]
struct PluginState {
    stream_deck: StreamDeckClient,
    global_settings: std::sync::Arc<Mutex<GlobalSettings>>,
    actions: std::sync::Arc<Mutex<HashMap<String, ActionState>>>,
    rpc_client: std::sync::Arc<Mutex<Option<DiscordRpcClient>>>,
    icon_cache: std::sync::Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum PropertyInspectorRequest {
    LoadState,
    SaveCredentials {
        #[serde(rename = "clientId", alias = "client_id", default)]
        client_id: String,
        #[serde(rename = "clientSecret", alias = "client_secret", default)]
        client_secret: String,
        #[serde(rename = "redirectUri", alias = "redirect_uri", default)]
        redirect_uri: String,
    },
    ConnectDiscord {
        #[serde(rename = "clientId", alias = "client_id")]
        client_id: Option<String>,
        #[serde(rename = "clientSecret", alias = "client_secret")]
        client_secret: Option<String>,
        #[serde(rename = "redirectUri", alias = "redirect_uri")]
        redirect_uri: Option<String>,
    },
    LoadGuilds,
    LoadChannels {
        #[serde(rename = "guildId", alias = "guild_id")]
        guild_id: String,
    },
    SaveActionSettings {
        #[serde(rename = "guildId", alias = "guild_id", default)]
        guild_id: String,
        #[serde(rename = "guildName", alias = "guild_name", default)]
        guild_name: String,
        #[serde(rename = "guildIconUrl", alias = "guild_icon_url", default)]
        guild_icon_url: String,
        #[serde(rename = "channelId", alias = "channel_id", default)]
        channel_id: String,
        #[serde(rename = "channelName", alias = "channel_name", default)]
        channel_name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse()?;
    let (stream_deck, mut reader) = StreamDeckClient::connect(
        args.port,
        args.plugin_uuid.clone(),
        args.register_event.clone(),
    )
    .await
    .context("failed to connect to Stream Deck/OpenDeck")?;

    stream_deck.get_global_settings().await?;

    let state = PluginState {
        stream_deck,
        global_settings: std::sync::Arc::new(Mutex::new(GlobalSettings::default())),
        actions: std::sync::Arc::new(Mutex::new(HashMap::new())),
        rpc_client: std::sync::Arc::new(Mutex::new(None)),
        icon_cache: std::sync::Arc::new(Mutex::new(HashMap::new())),
    };

    while let Some(message) = reader.next_json().await? {
        if let Err(error) = state.handle_message(message).await {
            eprintln!("plugin error: {error:#}");
        }
    }

    Ok(())
}

impl PluginState {
    async fn handle_message(&self, message: Value) -> Result<()> {
        let event = message
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or_default();

        match event {
            "willAppear" | "didReceiveSettings" => {
                self.handle_action_settings(&message).await?;
            }
            "didReceiveGlobalSettings" => {
                let settings = serde_json::from_value::<GlobalSettings>(
                    message
                        .get("payload")
                        .and_then(|payload| payload.get("settings"))
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                )
                .unwrap_or_default();

                *self.global_settings.lock().await = settings;
            }
            "keyDown" => {
                self.handle_key_down(&message).await?;
            }
            "sendToPlugin" => {
                if let Err(error) = self.handle_property_inspector_message(&message).await {
                    self.send_pi_error(&message, &error.to_string()).await?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_action_settings(&self, message: &Value) -> Result<()> {
        let context = message
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing context"))?
            .to_owned();
        let settings = serde_json::from_value::<VoiceChannelSettings>(
            message
                .get("payload")
                .and_then(|payload| payload.get("settings"))
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .unwrap_or_default();

        self.actions.lock().await.insert(
            context.clone(),
            ActionState {
                action_uuid: message
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                settings: settings.clone(),
                connected: false,
            },
        );

        self.stream_deck
            .set_title(&context, &settings.button_title())
            .await?;

        if let Err(error) = self.update_key_image(&context, &settings).await {
            eprintln!("failed to set key image: {error:#}");
        }

        Ok(())
    }

    async fn handle_key_down(&self, message: &Value) -> Result<()> {
        let context = message
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing context"))?;
        let action_uuid = message
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let action_state = self
            .actions
            .lock()
            .await
            .get(context)
            .cloned()
            .unwrap_or_else(|| ActionState {
                action_uuid: action_uuid.to_owned(),
                settings: serde_json::from_value::<VoiceChannelSettings>(
                    message
                        .get("payload")
                        .and_then(|payload| payload.get("settings"))
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                )
                .unwrap_or_default(),
                connected: false,
            });

        self
            .send_pi_log(
                action_uuid,
                context,
                &format!(
                    "keyDown: guild_id='{}' channel_id='{}' connected={}",
                    action_state.settings.guild_id,
                    action_state.settings.channel_id,
                    action_state.connected
                ),
            )
            .await?;

        if action_state.settings.channel_id.trim().is_empty() {
            self.send_pi_log(action_uuid, context, "keyDown aborted: no channel selected")
                .await?;
            self.stream_deck.show_alert(context).await?;
            return Ok(());
        }

        self.send_pi_log(action_uuid, context, "Attempting SELECT_VOICE_CHANNEL")
            .await?;
        let toggle_result = if action_state.connected {
            self.rpc_disconnect_voice_channel().await
        } else {
            self.rpc_select_voice_channel(&action_state.settings.channel_id)
                .await
        };
        if let Err(error) = toggle_result {
            let error_message = format!("Voice toggle failed: {error:#}");
            eprintln!("{error_message}");
            self.show_action_error(context, Some(action_uuid), &error_message)
                .await?;
            return Ok(());
        }

        self.send_pi_log(action_uuid, context, "Voice toggle succeeded")
            .await?;

        if let Some(stored_state) = self.actions.lock().await.get_mut(context) {
            stored_state.connected = !action_state.connected;
        }

        self.stream_deck
            .set_title(context, &action_state.settings.button_title())
            .await?;

        Ok(())
    }

    async fn handle_property_inspector_message(&self, message: &Value) -> Result<()> {
        let context = message
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing context"))?;
        let action_uuid = message
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = message
            .get("payload")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let request: PropertyInspectorRequest = serde_json::from_value(payload)
            .context("invalid property inspector request")?;

        match request {
            PropertyInspectorRequest::LoadState => {
                let settings = self.global_settings.lock().await.clone();
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "globalState",
                        "settings": {
                            "clientId": settings.client_id,
                            "clientSecret": settings.client_secret,
                            "redirectUri": settings.redirect_uri,
                            "isAuthorized": !settings.access_token.is_empty()
                        }
                    }),
                )
                .await?;
            }
            PropertyInspectorRequest::SaveCredentials {
                client_id,
                client_secret,
                redirect_uri,
            } => {
                let mut settings = self.global_settings.lock().await.clone();
                let mut changed = false;
                if settings.client_id != client_id {
                    changed = true;
                }
                settings.client_id = client_id;
                if settings.client_secret != client_secret {
                    changed = true;
                }
                settings.client_secret = client_secret;
                if settings.redirect_uri != redirect_uri {
                    changed = true;
                }
                settings.redirect_uri = redirect_uri;
                if changed {
                    settings.access_token.clear();
                }
                self.persist_global_settings(settings.clone()).await?;
                if changed {
                    self.clear_rpc_client().await;
                }
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "status",
                        "level": "success",
                        "message": "Credentials saved."
                    }),
                )
                .await?;
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "globalState",
                        "settings": {
                            "clientId": settings.client_id,
                            "clientSecret": settings.client_secret,
                            "redirectUri": settings.redirect_uri,
                            "isAuthorized": !settings.access_token.is_empty()
                        }
                    }),
                )
                .await?;
            }
            PropertyInspectorRequest::ConnectDiscord {
                client_id,
                client_secret,
                redirect_uri,
            } => {
                let changed = self
                    .update_credentials(client_id, client_secret, redirect_uri)
                    .await?;
                if changed {
                    self.clear_rpc_client().await;
                }
                let guilds = self.rpc_get_guilds().await?;
                let settings = self.global_settings.lock().await.clone();
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "guilds",
                        "guilds": guilds
                    }),
                )
                .await?;
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "status",
                        "level": "success",
                        "message": "Discord authorization succeeded."
                    }),
                )
                .await?;
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "globalState",
                        "settings": {
                            "clientId": settings.client_id,
                            "clientSecret": settings.client_secret,
                            "redirectUri": settings.redirect_uri,
                            "isAuthorized": !settings.access_token.is_empty()
                        }
                    }),
                )
                .await?;
            }
            PropertyInspectorRequest::LoadGuilds => {
                let guilds = self.rpc_get_guilds().await?;
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "guilds",
                        "guilds": guilds
                    }),
                )
                .await?;
            }
            PropertyInspectorRequest::LoadChannels { guild_id } => {
                let channels = self.rpc_get_channels(&guild_id).await?;
                self.send_pi_payload(
                    action_uuid,
                    context,
                    json!({
                        "type": "channels",
                        "guildId": guild_id,
                        "channels": channels
                    }),
                )
                .await?;
            }
            PropertyInspectorRequest::SaveActionSettings {
                guild_id,
                guild_name,
                guild_icon_url,
                channel_id,
                channel_name,
            } => {
                let settings_to_persist = {
                    let mut actions = self.actions.lock().await;
                    let action_state = actions.entry(context.to_owned()).or_default();
                    action_state.settings.guild_id = guild_id;
                    action_state.settings.guild_name = guild_name;
                    action_state.settings.guild_icon_url = guild_icon_url;
                    action_state.settings.channel_id = channel_id;
                    action_state.settings.channel_name = channel_name;
                    action_state.connected = false;
                    action_state.settings.clone()
                };

                self.stream_deck
                    .set_settings(context, &settings_to_persist)
                    .await?;
                self.send_pi_log(
                    action_uuid,
                    context,
                    &format!(
                        "saveActionSettings: guild_id='{}' guild_icon_url='{}' channel_id='{}' channel_name='{}'",
                        settings_to_persist.guild_id,
                        settings_to_persist.guild_icon_url,
                        settings_to_persist.channel_id,
                        settings_to_persist.channel_name
                    ),
                )
                .await?;
                self.stream_deck
                    .set_title(context, &settings_to_persist.button_title())
                    .await?;
                match self.update_key_image(context, &settings_to_persist).await {
                    Ok(true) => {
                        self.send_pi_log(action_uuid, context, "guild icon update applied")
                            .await?;
                    }
                    Ok(false) => {
                        self.send_pi_log(
                            action_uuid,
                            context,
                            "guild icon update skipped: selected server has no icon URL",
                        )
                        .await?;
                    }
                    Err(error) => {
                        self.send_pi_log(
                            action_uuid,
                            context,
                            &format!("guild icon update failed: {error}"),
                        )
                        .await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn update_credentials(
        &self,
        client_id: Option<String>,
        client_secret: Option<String>,
        redirect_uri: Option<String>,
    ) -> Result<bool> {
        if client_id.is_none() && client_secret.is_none() && redirect_uri.is_none() {
            return Ok(false);
        }

        let mut settings = self.global_settings.lock().await.clone();
        let mut changed = false;
        if let Some(client_id) = client_id {
            if settings.client_id != client_id {
                changed = true;
            }
            settings.client_id = client_id;
        }
        if let Some(client_secret) = client_secret {
            if settings.client_secret != client_secret {
                changed = true;
            }
            settings.client_secret = client_secret;
        }
        if let Some(redirect_uri) = redirect_uri {
            if settings.redirect_uri != redirect_uri {
                changed = true;
            }
            settings.redirect_uri = redirect_uri;
        }
        if changed {
            settings.access_token.clear();
        }
        self.persist_global_settings(settings).await?;
        Ok(changed)
    }

    async fn connect_authorized_client(&self) -> Result<DiscordRpcClient> {
        let settings = self.global_settings.lock().await.clone();
        if !settings.has_credentials() {
            bail!("Discord client ID and secret are required");
        }
        if !settings.has_redirect_uri() {
            bail!("Redirect URI is required and must match Discord Developer Portal OAuth2 redirect")
        }

        let (client, token) = DiscordRpcClient::connect_authorized(
            &settings.client_id,
            &settings.client_secret,
            &settings.redirect_uri,
            settings.access_token_if_present(),
        )
        .await?;
        self.persist_access_token(&token).await?;
        Ok(client)
    }

    async fn clear_rpc_client(&self) {
        *self.rpc_client.lock().await = None;
    }

    fn is_retryable_rpc_error(error: &anyhow::Error) -> bool {
        let message = error.to_string();
        message.contains("Discord RPC socket closed")
            || message.contains("failed to read Discord IPC payload")
            || message.contains("failed to write Discord IPC payload")
            || message.contains("Discord RPC error 4006")
            || message.contains("Discord RPC error 4009")
    }

    async fn ensure_connected_rpc_client(&self) -> Result<()> {
        if self.rpc_client.lock().await.is_some() {
            return Ok(());
        }

        let client = self.connect_authorized_client().await?;
        let mut rpc_client = self.rpc_client.lock().await;
        if rpc_client.is_none() {
            *rpc_client = Some(client);
        }
        Ok(())
    }

    async fn rpc_get_guilds(&self) -> Result<Vec<GuildSummary>> {
        for attempt in 0..2 {
            self.ensure_connected_rpc_client().await?;
            let result = {
                let mut rpc_client = self.rpc_client.lock().await;
                let client = rpc_client
                    .as_mut()
                    .ok_or_else(|| anyhow!("Discord RPC client is unavailable"))?;
                client.get_guilds().await
            };

            match result {
                Ok(guilds) => return Ok(guilds),
                Err(error) => {
                    if attempt == 0 && Self::is_retryable_rpc_error(&error) {
                        self.clear_rpc_client().await;
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        bail!("failed to get Discord servers")
    }

    async fn rpc_get_channels(&self, guild_id: &str) -> Result<Vec<ChannelSummary>> {
        for attempt in 0..2 {
            self.ensure_connected_rpc_client().await?;
            let result = {
                let mut rpc_client = self.rpc_client.lock().await;
                let client = rpc_client
                    .as_mut()
                    .ok_or_else(|| anyhow!("Discord RPC client is unavailable"))?;
                client.get_channels(guild_id).await
            };

            match result {
                Ok(channels) => return Ok(channels),
                Err(error) => {
                    if attempt == 0 && Self::is_retryable_rpc_error(&error) {
                        self.clear_rpc_client().await;
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        bail!("failed to get Discord channels")
    }

    async fn rpc_select_voice_channel(&self, channel_id: &str) -> Result<()> {
        for attempt in 0..2 {
            self.ensure_connected_rpc_client().await?;
            let result = {
                let mut rpc_client = self.rpc_client.lock().await;
                let client = rpc_client
                    .as_mut()
                    .ok_or_else(|| anyhow!("Discord RPC client is unavailable"))?;
                client.select_voice_channel(channel_id).await
            };

            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if attempt == 0 && Self::is_retryable_rpc_error(&error) {
                        self.clear_rpc_client().await;
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        bail!("failed to join Discord voice channel")
    }

    async fn rpc_disconnect_voice_channel(&self) -> Result<()> {
        for attempt in 0..2 {
            self.ensure_connected_rpc_client().await?;
            let result = {
                let mut rpc_client = self.rpc_client.lock().await;
                let client = rpc_client
                    .as_mut()
                    .ok_or_else(|| anyhow!("Discord RPC client is unavailable"))?;
                client.disconnect_voice_channel().await
            };

            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if attempt == 0 && Self::is_retryable_rpc_error(&error) {
                        self.clear_rpc_client().await;
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        bail!("failed to leave Discord voice channel")
    }

    async fn persist_access_token(&self, access_token: &str) -> Result<()> {
        let mut settings = self.global_settings.lock().await.clone();
        if settings.access_token == access_token {
            return Ok(());
        }
        settings.access_token = access_token.to_owned();
        self.persist_global_settings(settings).await
    }

    async fn persist_global_settings(&self, settings: GlobalSettings) -> Result<()> {
        *self.global_settings.lock().await = settings.clone();
        self.stream_deck.set_global_settings(&settings).await
    }

    async fn send_pi_payload(&self, action_uuid: &str, context: &str, payload: Value) -> Result<()> {
        self.stream_deck
            .send_to_property_inspector(action_uuid, context, &payload)
            .await
    }

    async fn send_pi_error(&self, message: &Value, error_message: &str) -> Result<()> {
        let context = message
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing context for property inspector error"))?;
        let action_uuid = message
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default();

        self.send_pi_payload(
            action_uuid,
            context,
            json!({
                "type": "status",
                "level": "error",
                "message": error_message
            }),
        )
        .await
    }

    async fn send_pi_log(&self, action_uuid: &str, context: &str, message: &str) -> Result<()> {
        if action_uuid.is_empty() {
            return Ok(());
        }

        self.send_pi_payload(
            action_uuid,
            context,
            json!({
                "type": "log",
                "message": message,
            }),
        )
        .await
    }

    async fn update_key_image(&self, context: &str, settings: &VoiceChannelSettings) -> Result<bool> {
        if settings.guild_icon_url.trim().is_empty() {
            return Ok(false);
        }

        let image = self
            .get_or_fetch_guild_icon_data_uri(&settings.guild_icon_url)
            .await?;
        self.stream_deck.set_image(context, &image).await?;
        Ok(true)
    }

    fn normalize_guild_icon_download_url(icon_url: &str) -> String {
        let (base, _) = icon_url.split_once('?').unwrap_or((icon_url, ""));
        let lower = base.to_ascii_lowercase();

        let mut normalized = if lower.ends_with(".webp")
            || lower.ends_with(".gif")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
        {
            let split_index = base.rfind('.').unwrap_or(base.len());
            format!("{}.png", &base[..split_index])
        } else {
            base.to_owned()
        };

        normalized.push_str("?size=128");
        normalized
    }

    async fn get_or_fetch_guild_icon_data_uri(&self, icon_url: &str) -> Result<String> {
        if let Some(cached) = self.icon_cache.lock().await.get(icon_url).cloned() {
            return Ok(cached);
        }

        let download_url = Self::normalize_guild_icon_download_url(icon_url);

        let response = Client::builder()
            .user_agent("discord-opendeck-plugin/0.1.0")
            .build()
            .context("failed to build icon fetch client")?
            .get(&download_url)
            .send()
            .await
            .context("failed to download guild icon")?
            .error_for_status()
            .context("guild icon request was rejected")?;
        let bytes = response
            .bytes()
            .await
            .context("failed to read guild icon response body")?;

        let data_uri = format!("data:image/png;base64,{}", BASE64_STANDARD.encode(&bytes));

        self.icon_cache
            .lock()
            .await
            .insert(icon_url.to_owned(), data_uri.clone());

        Ok(data_uri)
    }

    async fn show_action_error(
        &self,
        context: &str,
        action_uuid_from_event: Option<&str>,
        error_message: &str,
    ) -> Result<()> {
        self.stream_deck.show_alert(context).await?;
        let resolved_action_uuid = action_uuid_from_event
            .filter(|uuid| !uuid.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                self.actions
                    .try_lock()
                    .ok()
                    .and_then(|actions| actions.get(context).map(|state| state.action_uuid.clone()))
            })
            .unwrap_or_default();

        if !resolved_action_uuid.is_empty() {
            let _ = self
                .stream_deck
                .send_to_property_inspector(
                    &resolved_action_uuid,
                    context,
                    &json!({
                        "type": "status",
                        "level": "error",
                        "message": error_message,
                    }),
                )
                .await;

            let _ = self
                .send_pi_log(&resolved_action_uuid, context, error_message)
                .await;
        }
        Ok(())
    }
}

struct Args {
    port: u16,
    plugin_uuid: String,
    register_event: String,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut port = None;
        let mut plugin_uuid = None;
        let mut register_event = None;

        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let Some(value) = args.next() else {
                bail!("missing value for argument: {flag}");
            };
            match flag.as_str() {
                "-port" => port = Some(value.parse::<u16>().context("invalid port")?),
                "-pluginUUID" => plugin_uuid = Some(value),
                "-registerEvent" => register_event = Some(value),
                "-info" => {}
                _ => {}
            }
        }

        Ok(Self {
            port: port.ok_or_else(|| anyhow!("missing -port"))?,
            plugin_uuid: plugin_uuid.ok_or_else(|| anyhow!("missing -pluginUUID"))?,
            register_event: register_event.ok_or_else(|| anyhow!("missing -registerEvent"))?,
        })
    }
}

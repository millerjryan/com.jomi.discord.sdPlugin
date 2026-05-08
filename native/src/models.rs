use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSettings {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub access_token: String,
}

impl GlobalSettings {
    pub fn has_credentials(&self) -> bool {
        !self.client_id.trim().is_empty() && !self.client_secret.trim().is_empty()
    }

    pub fn has_redirect_uri(&self) -> bool {
        !self.redirect_uri.trim().is_empty()
    }

    pub fn access_token_if_present(&self) -> Option<&str> {
        let token = self.access_token.trim();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceChannelSettings {
    #[serde(default)]
    pub guild_id: String,
    #[serde(default)]
    pub guild_name: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub channel_name: String,
    #[serde(default)]
    pub guild_icon_url: String,
}

impl VoiceChannelSettings {
    pub fn button_title(&self) -> String {
        if self.channel_name.trim().is_empty() {
            return "Voice\nChannel".to_owned();
        }

        let mut title = self.channel_name.trim().to_owned();
        if title.len() > 18 {
            title.truncate(18);
        }
        title.replace(' ', "\n")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuildSummary {
    pub id: String,
    pub name: String,
    #[serde(default, rename = "iconUrl", alias = "icon_url")]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

impl GuildSummary {
    pub fn normalized_icon_url(&self) -> Option<String> {
        if let Some(url) = self.icon_url.as_ref().filter(|value| !value.is_empty()) {
            return Some(url.clone());
        }

        self.icon
            .as_ref()
            .filter(|value| !value.is_empty())
            .map(|hash| format!("https://cdn.discordapp.com/icons/{}/{}.png?size=128", self.id, hash))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: i64,
}

impl ChannelSummary {
    pub fn is_voice_like(&self) -> bool {
        matches!(self.kind, 2 | 13)
    }
}

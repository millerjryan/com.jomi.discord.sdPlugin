use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt, stream::{SplitSink, SplitStream}};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

#[derive(Clone)]
pub struct StreamDeckClient {
    plugin_uuid: String,
    writer: std::sync::Arc<Mutex<SplitSink<WsStream, Message>>>,
}

pub struct StreamDeckReader {
    reader: SplitStream<WsStream>,
}

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

impl StreamDeckClient {
    pub async fn connect(
        port: u16,
        plugin_uuid: String,
        register_event: String,
    ) -> Result<(Self, StreamDeckReader)> {
        let url = format!("ws://127.0.0.1:{port}");
        let (socket, _) = connect_async(url)
            .await
            .context("unable to open Stream Deck websocket")?;
        let (writer, reader) = socket.split();
        let client = Self {
            plugin_uuid: plugin_uuid.clone(),
            writer: std::sync::Arc::new(Mutex::new(writer)),
        };

        client
            .send(json!({
                "event": register_event,
                "uuid": plugin_uuid,
            }))
            .await?;

        Ok((client, StreamDeckReader { reader }))
    }

    pub async fn get_global_settings(&self) -> Result<()> {
        self.send(json!({
            "event": "getGlobalSettings",
            "context": self.plugin_uuid,
        }))
        .await
    }

    pub async fn set_global_settings<T>(&self, payload: &T) -> Result<()>
    where
        T: Serialize,
    {
        self.send(json!({
            "event": "setGlobalSettings",
            "context": self.plugin_uuid,
            "payload": payload,
        }))
        .await
    }

    pub async fn set_settings<T>(&self, context: &str, payload: &T) -> Result<()>
    where
        T: Serialize,
    {
        self.send(json!({
            "event": "setSettings",
            "context": context,
            "payload": payload,
        }))
        .await
    }

    pub async fn set_title(&self, context: &str, title: &str) -> Result<()> {
        self.send(json!({
            "event": "setTitle",
            "context": context,
            "payload": {
                "title": title,
                "target": 0,
            }
        }))
        .await
    }

    pub async fn set_image(&self, context: &str, image: &str) -> Result<()> {
        self.send(json!({
            "event": "setImage",
            "context": context,
            "payload": {
                "image": image,
                "target": 0,
                "state": 0,
            }
        }))
        .await
    }

    pub async fn set_state(&self, context: &str, state: u8) -> Result<()> {
        self.send(json!({
            "event": "setState",
            "context": context,
            "payload": {
                "state": state,
            }
        }))
        .await
    }

    pub async fn show_alert(&self, context: &str) -> Result<()> {
        self.send(json!({
            "event": "showAlert",
            "context": context,
        }))
        .await
    }

    pub async fn send_to_property_inspector<T>(
        &self,
        action_uuid: &str,
        context: &str,
        payload: &T,
    ) -> Result<()>
    where
        T: Serialize,
    {
        self.send(json!({
            "event": "sendToPropertyInspector",
            "action": action_uuid,
            "context": context,
            "payload": payload,
        }))
        .await
    }

    async fn send(&self, payload: Value) -> Result<()> {
        let message = Message::Text(payload.to_string().into());
        let mut writer = self.writer.lock().await;
        writer
            .send(message)
            .await
            .context("failed to send Stream Deck payload")
    }
}

impl StreamDeckReader {
    pub async fn next_json(&mut self) -> Result<Option<Value>> {
        loop {
            let Some(message) = self.reader.next().await else {
                return Ok(None);
            };
            let message = message.context("failed to read Stream Deck message")?;
            match message {
                Message::Text(text) => {
                    let payload = serde_json::from_str::<Value>(text.as_ref())
                        .context("invalid Stream Deck JSON message")?;
                    return Ok(Some(payload));
                }
                Message::Binary(bytes) => {
                    let payload = serde_json::from_slice::<Value>(&bytes)
                        .context("invalid binary Stream Deck JSON message")?;
                    return Ok(Some(payload));
                }
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }
}

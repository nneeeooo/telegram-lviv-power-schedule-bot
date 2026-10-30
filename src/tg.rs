//! Tiny Telegram Bot API client: long polling (messages + button presses), photos with inline
//! keyboards, caption / media edits, text messages, deletes.

use anyhow::Result;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

pub struct Bot {
    http: reqwest::Client,
    base: String,
    /// Highest message id seen per chat (sent or received), persisted: private-chat ids are
    /// sequential, so 1..=max covers the whole chat for `clean_all`.
    seen: std::sync::Mutex<HashMap<i64, i64>>,
}

fn seen_path() -> PathBuf {
    let dir = PathBuf::from("cache");
    std::fs::create_dir_all(&dir).ok();
    dir.join("msgids.json")
}

#[derive(Debug)]
pub enum TgError {
    /// 429: wait this many seconds.
    RetryAfter(u64),
    /// The message is gone (deleted by the user) or cannot be edited.
    Gone(String),
    /// Edit with identical content; harmless.
    NotModified,
    Other(String),
}

impl TgError {
    pub fn text(&self) -> String {
        match self {
            TgError::RetryAfter(s) => format!("retry after {s} s"),
            TgError::NotModified => "not modified".into(),
            TgError::Gone(s) | TgError::Other(s) => s.clone(),
        }
    }
}

#[derive(Deserialize)]
struct Resp<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
    parameters: Option<RespParams>,
}

#[derive(Deserialize)]
struct RespParams {
    retry_after: Option<u64>,
}

#[derive(Deserialize, Debug)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub callback_query: Option<Callback>,
}

/// Inline-keyboard button press.
#[derive(Deserialize, Debug)]
pub struct Callback {
    pub id: String,
    pub message: Option<Message>,
    pub data: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Message {
    pub message_id: i64,
    pub chat: Chat,
    pub text: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct Chat {
    pub id: i64,
}

/// Inline keyboard from rows of (label, callback_data).
pub fn inline(rows: Vec<Vec<(String, String)>>) -> Value {
    let rows: Vec<Vec<Value>> = rows
        .into_iter()
        .map(|r| r.into_iter().map(|(t, d)| json!({"text": t, "callback_data": d})).collect())
        .collect();
    json!({ "inline_keyboard": rows })
}

pub fn btn(t: impl Into<String>, d: impl Into<String>) -> (String, String) {
    (t.into(), d.into())
}

impl Bot {
    pub fn new(token: &str) -> Self {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(70)).build().expect("http client");
        let seen = std::fs::read(seen_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Bot { http, base: format!("https://api.telegram.org/bot{token}"), seen: std::sync::Mutex::new(seen) }
    }

    /// Remember a message id in a chat (anything the bot sent or received).
    pub fn note(&self, chat: i64, id: i64) {
        let mut m = self.seen.lock().unwrap();
        let e = m.entry(chat).or_insert(0);
        if id > *e {
            *e = id;
            if let Ok(s) = serde_json::to_string(&*m) {
                std::fs::write(seen_path(), s).ok();
            }
        }
    }

    /// Delete everything deletable in every known chat (bot + user messages; Telegram only
    /// allows messages younger than 48 h). Returns (chats, ids tried).
    pub async fn clean_all(&self) -> (usize, i64) {
        let chats: Vec<(i64, i64)> = self.seen.lock().unwrap().iter().map(|(c, m)| (*c, *m)).collect();
        let mut tried = 0;
        for (chat, max) in &chats {
            let ids: Vec<i64> = (1..=*max + 2).collect();
            for batch in ids.chunks(100) {
                tried += batch.len() as i64;
                if let Err(e) = self.post::<Value>("deleteMessages", json!({"chat_id": chat, "message_ids": batch})).await {
                    if let TgError::RetryAfter(s) = e {
                        tokio::time::sleep(Duration::from_secs(s)).await;
                    }
                }
            }
        }
        (chats.len(), tried)
    }

    async fn call<T: for<'de> Deserialize<'de>>(&self, req: reqwest::RequestBuilder) -> Result<T, TgError> {
        // without_url(): reqwest errors print the request URL, which contains the bot token.
        let resp = req.send().await.map_err(|e| TgError::Other(e.without_url().to_string()))?;
        let body: Resp<T> = resp.json().await.map_err(|e| TgError::Other(e.without_url().to_string()))?;
        if body.ok {
            return body.result.ok_or_else(|| TgError::Other("empty result".into()));
        }
        let desc = body.description.unwrap_or_default();
        if let Some(s) = body.parameters.and_then(|p| p.retry_after) {
            return Err(TgError::RetryAfter(s));
        }
        if desc.contains("message is not modified") {
            return Err(TgError::NotModified);
        }
        if desc.contains("message to edit not found")
            || desc.contains("message can't be edited")
            || desc.contains("chat not found")
            || desc.contains("bot was blocked")
        {
            return Err(TgError::Gone(desc));
        }
        Err(TgError::Other(desc))
    }

    async fn post<T: for<'de> Deserialize<'de>>(&self, method: &str, body: Value) -> Result<T, TgError> {
        self.call(self.http.post(format!("{}/{method}", self.base)).json(&body)).await
    }

    pub async fn updates(&self, offset: i64) -> Result<Vec<Update>, TgError> {
        let u: Vec<Update> = self
            .post("getUpdates", json!({"offset": offset, "timeout": 50, "allowed_updates": ["message", "callback_query"]}))
            .await?;
        for x in &u {
            if let Some(m) = x.message.as_ref().or(x.callback_query.as_ref().and_then(|c| c.message.as_ref())) {
                self.note(m.chat.id, m.message_id);
            }
        }
        Ok(u)
    }

    /// Sends a text message, returns its message_id.
    pub async fn send_text(&self, chat: i64, text: &str) -> Result<i64, TgError> {
        let m = self.post::<Message>("sendMessage", json!({"chat_id": chat, "text": text})).await?;
        self.note(chat, m.message_id);
        Ok(m.message_id)
    }

    /// Removes a leftover reply keyboard (from older bot versions): send + delete a service message.
    pub async fn remove_reply_keyboard(&self, chat: i64) {
        let sent = self
            .post::<Message>("sendMessage", json!({"chat_id": chat, "text": "…", "reply_markup": {"remove_keyboard": true}}))
            .await;
        if let Ok(m) = sent {
            self.note(chat, m.message_id);
            self.delete(chat, m.message_id).await.ok();
        }
    }

    pub async fn delete(&self, chat: i64, msg: i64) -> Result<(), TgError> {
        self.post::<Value>("deleteMessage", json!({"chat_id": chat, "message_id": msg})).await.map(|_| ())
    }

    /// Stops the button's loading spinner (optionally with a toast).
    pub async fn answer(&self, callback_id: &str, toast: Option<&str>) {
        let mut body = json!({"callback_query_id": callback_id});
        if let Some(t) = toast {
            body["text"] = json!(t);
        }
        self.post::<Value>("answerCallbackQuery", body).await.ok();
    }

    fn png(png: Vec<u8>) -> Part {
        Part::bytes(png).file_name("card.png").mime_str("image/png").expect("mime")
    }

    /// Sends a photo with caption and inline keyboard, returns its message_id.
    pub async fn send_photo(&self, chat: i64, png: Vec<u8>, caption: &str, kb: &Value) -> Result<i64, TgError> {
        let form = Form::new()
            .text("chat_id", chat.to_string())
            .text("caption", caption.to_string())
            .text("reply_markup", kb.to_string())
            .part("photo", Self::png(png));
        let req = self.http.post(format!("{}/sendPhoto", self.base)).multipart(form);
        let m = self.call::<Message>(req).await?;
        self.note(chat, m.message_id);
        Ok(m.message_id)
    }

    /// Replaces picture, caption and keyboard of an existing photo message.
    pub async fn edit_photo(&self, chat: i64, msg: i64, png: Vec<u8>, caption: &str, kb: &Value) -> Result<(), TgError> {
        let media = json!({"type": "photo", "media": "attach://card", "caption": caption});
        let form = Form::new()
            .text("chat_id", chat.to_string())
            .text("message_id", msg.to_string())
            .text("media", media.to_string())
            .text("reply_markup", kb.to_string())
            .part("card", Self::png(png));
        let req = self.http.post(format!("{}/editMessageMedia", self.base)).multipart(form);
        self.call::<Value>(req).await.map(|_| ())
    }

    /// Replaces the text of a text message (quiet: no notification).
    // (used by both bots)
    pub async fn edit_text(&self, chat: i64, msg: i64, text: &str) -> Result<(), TgError> {
        self.post::<Value>("editMessageText", json!({"chat_id": chat, "message_id": msg, "text": text}))
            .await
            .map(|_| ())
    }

    /// Replaces only caption + keyboard (menus on top of the card, no re-upload).
    pub async fn edit_caption(&self, chat: i64, msg: i64, caption: &str, kb: &Value) -> Result<(), TgError> {
        self.post::<Value>(
            "editMessageCaption",
            json!({"chat_id": chat, "message_id": msg, "caption": caption, "reply_markup": kb}),
        )
        .await
        .map(|_| ())
    }
}

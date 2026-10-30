//! Per-chat state, persisted to cache/state.json so restarts keep editing the same messages.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct House {
    pub name: String,
    /// Full address as resolved, or "група X.Y" when added by group.
    pub address: String,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Chat {
    pub houses: Vec<House>,
    /// Message 1: the picture (schedule or frog) with the menu buttons; edited, never resent.
    #[serde(alias = "main_msg")]
    pub card_msg: Option<i64>,
    /// Message 2: the latest update info; on every change it is deleted and sent again.
    #[serde(alias = "status_msg")]
    pub info_msg: Option<i64>,
    /// Status dot of the info message last sent (🔴 / 🟠 / 🟢); a change resends it.
    #[serde(default)]
    pub info_dot: Option<String>,
    /// Fingerprint of the schedule + houses last drawn.
    pub drawn: Option<String>,
    /// Layout version; chats from older versions are migrated (fresh two-message layout) at boot.
    #[serde(default)]
    pub v: u32,
    /// Whether the last drawn state had a schedule (to word "appeared" vs "updated").
    pub had_schedule: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub chats: HashMap<i64, Chat>,
}

fn path() -> PathBuf {
    let dir = PathBuf::from("cache");
    std::fs::create_dir_all(&dir).ok();
    dir.join("state.json")
}

impl State {
    pub fn load() -> State {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            std::fs::write(path(), s).ok();
        }
    }
}

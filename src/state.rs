//! Per-chat state, persisted to PostgreSQL (or cache/state.json if DATABASE_URL is not set).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio_postgres::Client;

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

pub struct State {
    pub chats: HashMap<i64, Chat>,
    db: Option<Client>,
}

fn path() -> PathBuf {
    let dir = PathBuf::from("cache");
    std::fs::create_dir_all(&dir).ok();
    dir.join("state.json")
}

impl State {
    pub async fn load() -> State {
        if let Ok(url) = std::env::var("DATABASE_URL") {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                match tokio_postgres::connect(trimmed, tokio_postgres::NoTls).await {
                    Ok((client, connection)) => {
                        tokio::spawn(async move {
                            if let Err(e) = connection.await {
                                eprintln!("postgres connection error: {e}");
                            }
                        });
                        if let Err(e) = Self::init_db(&client).await {
                            eprintln!("failed to init postgres tables: {e}");
                        }
                        let mut state = Self::load_from_db(&client).await.unwrap_or_else(|e| {
                            eprintln!("failed to load from postgres: {e}");
                            State { chats: HashMap::new(), db: None }
                        });
                        state.migrate_json_if_needed(&client).await;
                        state.db = Some(client);
                        println!("loaded state from postgres ({} chat(s))", state.chats.len());
                        return state;
                    }
                    Err(e) => {
                        eprintln!("failed to connect to DATABASE_URL: {e}, falling back to json");
                    }
                }
            }
        }

        let chats = std::fs::read(path())
            .ok()
            .and_then(|b| {
                #[derive(Deserialize)]
                struct Wrapper { chats: HashMap<i64, Chat> }
                serde_json::from_slice::<Wrapper>(&b).map(|w| w.chats)
                    .or_else(|_| serde_json::from_slice::<HashMap<i64, Chat>>(&b))
                    .ok()
            })
            .unwrap_or_default();
        println!("loaded state from json ({} chat(s))", chats.len());
        State { chats, db: None }
    }

    async fn init_db(client: &Client) -> Result<(), tokio_postgres::Error> {
        client.batch_execute(
            "CREATE TABLE IF NOT EXISTS chats (
                chat_id BIGINT PRIMARY KEY,
                card_msg BIGINT,
                info_msg BIGINT,
                info_dot TEXT,
                drawn TEXT,
                v INT NOT NULL DEFAULT 0,
                had_schedule BOOLEAN NOT NULL DEFAULT false,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            CREATE TABLE IF NOT EXISTS houses (
                id BIGSERIAL PRIMARY KEY,
                chat_id BIGINT NOT NULL REFERENCES chats(chat_id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                address TEXT NOT NULL,
                groups TEXT[] NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            CREATE INDEX IF NOT EXISTS idx_houses_chat_id ON houses(chat_id);"
        ).await
    }

    async fn load_from_db(client: &Client) -> Result<Self, tokio_postgres::Error> {
        let chat_rows = client.query("SELECT chat_id, card_msg, info_msg, info_dot, drawn, v, had_schedule FROM chats", &[]).await?;
        let mut chats = HashMap::new();
        for row in chat_rows {
            let chat_id: i64 = row.get(0);
            chats.insert(chat_id, Chat {
                houses: Vec::new(),
                card_msg: row.get(1),
                info_msg: row.get(2),
                info_dot: row.get(3),
                drawn: row.get(4),
                v: row.get::<_, i32>(5) as u32,
                had_schedule: row.get(6),
            });
        }
        let house_rows = client.query("SELECT chat_id, name, address, groups FROM houses ORDER BY id ASC", &[]).await?;
        for row in house_rows {
            let chat_id: i64 = row.get(0);
            let house = House {
                name: row.get(1),
                address: row.get(2),
                groups: row.get(3),
            };
            if let Some(chat) = chats.get_mut(&chat_id) {
                chat.houses.push(house);
            }
        }
        Ok(State { chats, db: None })
    }

    async fn migrate_json_if_needed(&mut self, client: &Client) {
        let p = path();
        if !p.exists() {
            return;
        }
        let Ok(bytes) = std::fs::read(&p) else { return };
        #[derive(Deserialize)]
        struct Legacy { chats: HashMap<i64, Chat> }
        let legacy_chats = serde_json::from_slice::<Legacy>(&bytes)
            .map(|l| l.chats)
            .or_else(|_| serde_json::from_slice::<HashMap<i64, Chat>>(&bytes))
            .unwrap_or_default();

        if !legacy_chats.is_empty() {
            let mut imported = 0;
            for (chat_id, chat) in legacy_chats {
                if !self.chats.contains_key(&chat_id) {
                    self.chats.insert(chat_id, chat);
                    imported += 1;
                }
            }
            if imported > 0 {
                if let Err(e) = self.sync_all_to_db(client).await {
                    eprintln!("migration to db error: {e}");
                    return;
                }
            }
            let migrated_path = p.with_extension("json.migrated");
            let _ = std::fs::rename(&p, migrated_path);
            println!("migrated {imported} legacy chat(s) from state.json to postgres");
        }
    }

    pub async fn save(&self) {
        if let Some(client) = &self.db {
            if let Err(e) = self.sync_all_to_db(client).await {
                eprintln!("error saving state to postgres: {e}");
            }
        } else {
            #[derive(Serialize)]
            struct Wrapper<'a> { chats: &'a HashMap<i64, Chat> }
            if let Ok(s) = serde_json::to_string_pretty(&Wrapper { chats: &self.chats }) {
                std::fs::write(path(), s).ok();
            }
        }
    }

    async fn sync_all_to_db(&self, client: &Client) -> Result<(), tokio_postgres::Error> {
        // Delete chats no longer in memory
        let existing_ids = client.query("SELECT chat_id FROM chats", &[]).await?;
        for row in existing_ids {
            let id: i64 = row.get(0);
            if !self.chats.contains_key(&id) {
                client.execute("DELETE FROM chats WHERE chat_id = $1", &[&id]).await?;
            }
        }

        // Upsert all chats and their houses
        for (&chat_id, chat) in &self.chats {
            client.execute(
                "INSERT INTO chats (chat_id, card_msg, info_msg, info_dot, drawn, v, had_schedule, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
                 ON CONFLICT (chat_id) DO UPDATE SET
                    card_msg = EXCLUDED.card_msg,
                    info_msg = EXCLUDED.info_msg,
                    info_dot = EXCLUDED.info_dot,
                    drawn = EXCLUDED.drawn,
                    v = EXCLUDED.v,
                    had_schedule = EXCLUDED.had_schedule,
                    updated_at = NOW()",
                &[&chat_id, &chat.card_msg, &chat.info_msg, &chat.info_dot, &chat.drawn, &(chat.v as i32), &chat.had_schedule],
            ).await?;

            client.execute("DELETE FROM houses WHERE chat_id = $1", &[&chat_id]).await?;
            for h in &chat.houses {
                client.execute(
                    "INSERT INTO houses (chat_id, name, address, groups, created_at)
                     VALUES ($1, $2, $3, $4, NOW())",
                    &[&chat_id, &h.name, &h.address, &h.groups],
                ).await?;
            }
        }
        Ok(())
    }
}

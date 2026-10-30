//! Telegram bot for Lvivoblenergo hourly outage schedules (ГПВ).
//!
//! The chat always holds exactly two bot messages:
//! 1. the card: schedule picture for the tracked addresses (a celebrating frog when there is no
//!    schedule) with the menu buttons «Слідкувати за адресою» / «Мої адреси»; it is only edited;
//! 2. the info message: "Оновлення графіків (дата о час)" or "Без графіків (Перевірено о …)";
//!    on every schedule change it is deleted and sent again, so Telegram notifies.
//! Every CHECK_SECS (300 = 5 min) the schedule is fetched; the card + info change only when it changed.
//!
//! Config (.env): TELEGRAM_BOT_TOKEN, optional ALLOWED_CHAT_IDS=1,2 and CHECK_SECS=300.
//! Local checks: `--render out.png [group...]`, `--render-sample out.png [group...]` (latest
//! archived schedule), `--resolve "Львів, Городоцька, 171"`.

mod loe;
mod render;
mod state;
mod text;
mod tg;
mod ui;

use anyhow::{Context, Result};
use chrono::{Timelike, Utc};
use chrono_tz::Europe::Kyiv;
use loe::{Loe, Schedules};
use render::Renderer;
use state::{House, State};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tg::{Bot, TgError};
use tokio::sync::Mutex;

fn now_kyiv() -> chrono::DateTime<chrono_tz::Tz> {
    Utc::now().with_timezone(&Kyiv)
}

/// Fingerprint of what a chat would see: published schedule + its houses.
fn fingerprint(s: &Schedules, houses: &[House]) -> String {
    let h: Vec<String> = houses.iter().map(|h| format!("{}={}", h.name, h.groups.join(","))).collect();
    format!("{}\n##\n{}", s.fingerprint(), h.join(";"))
}

fn who(h: &House, g: &str) -> String {
    if h.name.contains(g) { h.name.clone() } else { format!("{} ({g})", h.name) }
}

/// Caption of the card on the main screen.
pub fn caption(s: &Schedules, houses: &[House]) -> String {
    if s.is_empty() {
        return "🎉 Графіків відключень немає".into();
    }
    let mut out = "💡 Графік відключень".to_string();
    if let Some(a) = s.as_of() {
        out += &format!(" · станом на {a}");
    }
    if let Some(d) = &s.today {
        for h in houses {
            for g in &h.groups {
                let w = d.groups.get(g).map(|w| render::windows_text(w)).unwrap_or_else(|| "немає в графіку".into());
                out += &format!("\n{}: {w}", who(h, g));
            }
        }
    }
    out
}

/// "18:56 17.10.2026" (LOE) -> "17.10 о 18:56".
fn as_of_label(a: &str) -> String {
    match a.split_once(' ') {
        Some((t, d)) => format!("{} о {t}", d.rsplit_once('.').map(|(dm, _)| dm).unwrap_or(d)),
        None => a.to_string(),
    }
}

/// Status dot for the tracked addresses: 🔴 outage right now, 🟠 outages in the schedule but
/// power is on now, 🟢 no outages (or no schedule). Without addresses: any group counts.
fn dot(s: &Schedules, houses: &[House]) -> &'static str {
    let n = now_kyiv();
    dot_at(s, houses, n.date_naive(), n.hour() * 60 + n.minute())
}

fn dot_at(s: &Schedules, houses: &[House], today: chrono::NaiveDate, now: u32) -> &'static str {
    let groups: Vec<&String> = houses.iter().flat_map(|h| h.groups.iter()).collect();
    let wins = |d: &loe::DaySchedule| -> Vec<loe::Window> {
        d.groups
            .iter()
            .filter(|(g, _)| groups.is_empty() || groups.contains(g))
            .flat_map(|(_, w)| w.iter().copied())
            .collect()
    };
    let today_wins = s.today.as_ref().filter(|d| d.date.map_or(true, |x| x == today)).map(wins).unwrap_or_default();
    if today_wins.iter().any(|&(a, b)| a <= now && now < b) {
        return "🔴";
    }
    let upcoming = today_wins.iter().any(|&(_, b)| b > now) || s.tomorrow.as_ref().map(|d| !wins(d).is_empty()).unwrap_or(false);
    if upcoming { "🟠" } else { "🟢" }
}

/// Message 2: dot + "Оновлення графіків (17.10 о 18:56)" or "Без графіків", and when checked.
fn info_text(s: &Schedules, houses: &[House]) -> String {
    let checked = format!("(Перевірено о {})", now_kyiv().format("%H:%M"));
    let d = dot(s, houses);
    if s.is_empty() {
        return format!("{d} Без графіків\n{checked}");
    }
    let at = s.as_of().map(as_of_label).unwrap_or_else(|| "дата невідома".into());
    format!("{d} Оновлення графіків\n({at})\n{checked}")
}

fn card_png(r: &Renderer, s: &Schedules, houses: &[House]) -> Vec<u8> {
    let n = now_kyiv();
    let label = n.format("%H:%M %d.%m").to_string();
    tokio::task::block_in_place(|| if s.is_empty() { r.frog(&label) } else { r.schedule(s, houses, n.date_naive(), &label) })
}

/// Current chat layout version (see state::Chat::v).
const LAYOUT: u32 = 2;

pub struct App {
    bot: Bot,
    http: reqwest::Client,
    loe: Loe,
    r: Renderer,
    state: Mutex<State>,
    last: Mutex<Option<Schedules>>,
    ui: Mutex<HashMap<i64, ui::Ui>>,
    /// Chats with an action in progress: extra clicks are answered "wait" instead of queueing.
    busy: Mutex<HashSet<i64>>,
}

impl App {
    async fn fetch(&self) -> Result<Schedules> {
        let s = loe::fetch_schedules(&self.http).await?;
        *self.last.lock().await = Some(s.clone());
        Ok(s)
    }

    async fn schedules(&self) -> Schedules {
        let cached = self.last.lock().await.clone();
        match cached {
            Some(s) => s,
            None => self.fetch().await.unwrap_or_default(),
        }
    }

    /// Bring both messages in line with the schedule. `force` redraws the card even if unchanged.
    async fn sync_chat(&self, chat: i64, force: bool) {
        let s = self.schedules().await;
        let (houses, card, info, drawn, old_dot) = {
            let st = self.state.lock().await;
            let Some(c) = st.chats.get(&chat) else { return };
            (c.houses.clone(), c.card_msg, c.info_msg, c.drawn.clone(), c.info_dot.clone())
        };
        let fp = fingerprint(&s, &houses);
        let changed = drawn.as_deref() != Some(fp.as_str());
        let text = info_text(&s, &houses);
        let new_dot = dot(&s, &houses).to_string();
        let dot_changed = old_dot.as_deref() != Some(new_dot.as_str());
        if !changed && !force && card.is_some() {
            if let Some(id) = info {
                if dot_changed {
                    // Power went off / on: resend so Telegram notifies.
                    self.bot.delete(chat, id).await.ok();
                    let new = self.bot.send_text(chat, &text).await.ok();
                    let mut st = self.state.lock().await;
                    if let Some(c) = st.chats.get_mut(&chat) {
                        c.info_msg = new;
                        c.info_dot = Some(new_dot);
                    }
                    st.save().await;
                } else {
                    // Keep "(Перевірено …)" fresh without a notification.
                    self.bot.edit_text(chat, id, &text).await.ok();
                }
            }
            return;
        }
        let png = card_png(&self.r, &s, &houses);
        let (cap, kb) = self.face(chat).await;
        let card = match card {
            Some(m) => match self.bot.edit_photo(chat, m, png.clone(), &cap, &kb).await {
                Ok(()) | Err(TgError::NotModified) => Some(m),
                Err(TgError::Gone(_)) => self.send_card(chat, png, &cap, &kb).await,
                Err(e) => {
                    eprintln!("edit {chat}: {}", e.text());
                    return; // retry next tick
                }
            },
            None => self.send_card(chat, png, &cap, &kb).await,
        };
        // Message 2: delete + send again on every change (or when it is missing).
        let mut info_new = info;
        if changed || info.is_none() {
            if let Some(old) = info {
                self.bot.delete(chat, old).await.ok();
            }
            info_new = match self.bot.send_text(chat, &text).await {
                Ok(id) => Some(id),
                Err(e) => {
                    eprintln!("info {chat}: {}", e.text());
                    None
                }
            };
        }
        let mut st = self.state.lock().await;
        if let Some(c) = st.chats.get_mut(&chat) {
            c.card_msg = card;
            c.info_msg = info_new;
            c.drawn = Some(fp);
            c.info_dot = Some(new_dot);
            c.had_schedule = !s.is_empty();
        }
        st.save().await;
    }

    async fn send_card(&self, chat: i64, png: Vec<u8>, cap: &str, kb: &serde_json::Value) -> Option<i64> {
        match self.bot.send_photo(chat, png, cap, kb).await {
            Ok(id) => Some(id),
            Err(e) => {
                eprintln!("card {chat}: {}", e.text());
                None
            }
        }
    }

    /// Menu change: caption + buttons only.
    async fn redraw_caption(&self, chat: i64) {
        let card = self.state.lock().await.chats.get(&chat).and_then(|c| c.card_msg);
        let Some(card) = card else { return self.sync_chat(chat, true).await };
        let (cap, kb) = self.face(chat).await;
        match self.bot.edit_caption(chat, card, &cap, &kb).await {
            Ok(()) | Err(TgError::NotModified) => {}
            Err(TgError::Gone(_)) => {
                if let Some(c) = self.state.lock().await.chats.get_mut(&chat) {
                    c.card_msg = None;
                }
                self.sync_chat(chat, true).await;
            }
            Err(e) => eprintln!("caption {chat}: {}", e.text()),
        }
    }

    /// /start: replace both messages with fresh ones at the bottom of the chat.
    async fn start(&self, chat: i64) {
        let old = {
            let mut st = self.state.lock().await;
            let c = st.chats.entry(chat).or_default();
            let old = [c.card_msg.take(), c.info_msg.take()];
            c.drawn = None;
            st.save().await;
            old
        };
        for id in old.into_iter().flatten() {
            self.bot.delete(chat, id).await.ok();
        }
        self.ui.lock().await.insert(chat, ui::Ui::default());
        self.bot.remove_reply_keyboard(chat).await;
        if let Some(c) = self.state.lock().await.chats.get_mut(&chat) {
            c.v = LAYOUT;
        }
        self.sync_chat(chat, true).await;
    }

    async fn on_text(&self, chat: i64, msg: i64, text: &str) {
        self.bot.delete(chat, msg).await.ok(); // keep the chat at two messages
        match text.split(['@', ' ']).next().unwrap_or("") {
            // Buttons of the old reply keyboard (pre-0.2) also mean "start over".
            "/start" | "➕" | "📋" | "🔄" => self.start(chat).await,
            "/stop" => {
                let old = self.state.lock().await.chats.remove(&chat);
                self.state.lock().await.save().await;
                let ids: Vec<i64> = old.iter().flat_map(|c| [c.card_msg, c.info_msg]).flatten().collect();
                for id in ids {
                    self.bot.delete(chat, id).await.ok();
                }
                self.bot.send_text(chat, "Зупинив, адреси видалено. /start — почати знову.").await.ok();
            }
            _ => {
                let known = self.state.lock().await.chats.contains_key(&chat);
                if known { self.on_input(chat, text).await } else { self.start(chat).await }
            }
        }
    }

    /// Marks the chat busy; false when an action is already running for it.
    async fn begin(&self, chat: i64) -> bool {
        self.busy.lock().await.insert(chat)
    }

    async fn end(&self, chat: i64) {
        self.busy.lock().await.remove(&chat);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let http = reqwest::Client::builder()
        .user_agent("lviv-power-bot/0.2 (personal, non-commercial)")
        .timeout(Duration::from_secs(40))
        .build()?;
    let r = Renderer { text: text::Text::load()? };
    let loe = Loe::new(http.clone());

    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str);
    if mode == Some("--menu") {
        let otgs = loe.otgs().await?;
        println!("{} громад: {}", otgs.len(), otgs.iter().take(8).map(|o| o.name.as_str()).collect::<Vec<_>>().join(", "));
        let cities = loe.cities(&otgs[0]).await?;
        println!("{}: {}", otgs[0].name, cities.iter().take(8).map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "));
        return Ok(());
    }
    if mode == Some("--resolve") {
        let q = args.get(2).cloned().unwrap_or_default();
        let p: Vec<&str> = q.split(',').map(str::trim).collect();
        anyhow::ensure!(p.len() == 3, "--resolve \"Місто, Вулиця, Будинок\"");
        let (label, groups) = loe.resolve(p[0], p[1], p[2]).await?;
        println!("{label} -> {}", groups.join(", "));
        return Ok(());
    }
    if mode == Some("--render") || mode == Some("--render-sample") {
        let out = args.get(2).cloned().unwrap_or_else(|| "card.png".into());
        let s = if mode == Some("--render") { loe::fetch_schedules(&http).await? } else { loe::fetch_archive_sample(&http).await? };
        let houses: Vec<House> = args
            .iter()
            .skip(3)
            .map(|g| House { name: format!("Група {g}"), address: String::new(), groups: vec![g.clone()] })
            .collect();
        std::fs::write(&out, card_png(&r, &s, &houses))?;
        println!("{}\n-- info --\n{}\n-> {out}", caption(&s, &houses), info_text(&s, &houses));
        return Ok(());
    }

    let token = std::env::var("TELEGRAM_BOT_TOKEN").context("TELEGRAM_BOT_TOKEN is not set (.env)")?;
    let allowed: Vec<i64> = std::env::var("ALLOWED_CHAT_IDS").unwrap_or_default().split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let every = std::env::var("CHECK_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(300u64).max(30);
    let app = Arc::new(App {
        bot: Bot::new(&token),
        http,
        loe,
        r,
        state: Mutex::new(State::load().await),
        last: Mutex::new(None),
        ui: Mutex::new(HashMap::new()),
        busy: Mutex::new(HashSet::new()),
    });
    println!("power bot up, {} chat(s), check every {every} s", app.state.lock().await.chats.len());

    // Clean every chat (all users) before starting: delete all bot + user messages Telegram still
    // allows (< 48 h), then send each subscribed chat a fresh card + info pair.
    let known: Vec<(i64, i64)> = app
        .state
        .lock()
        .await
        .chats
        .iter()
        .map(|(c, x)| (*c, x.card_msg.into_iter().chain(x.info_msg).max().unwrap_or(0) + 30))
        .collect();
    for (chat, id) in &known {
        app.bot.note(*chat, *id);
    }
    let (n, tried) = app.bot.clean_all().await;
    println!("cleaned {n} chat(s), {tried} message ids");
    for (chat, _) in known {
        if let Some(c) = app.state.lock().await.chats.get_mut(&chat) {
            c.card_msg = None;
            c.info_msg = None;
        }
        app.start(chat).await;
    }
    app.state.lock().await.save().await;

    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(every));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                if let Err(e) = app.fetch().await {
                    eprintln!("schedule: {e:#}");
                    continue;
                }
                let chats: Vec<i64> = app.state.lock().await.chats.keys().copied().collect();
                for chat in chats {
                    // Skip chats mid-click; they are synced by their own action.
                    if app.begin(chat).await {
                        app.sync_chat(chat, false).await;
                        app.end(chat).await;
                    }
                }
            }
        });
    }

    let mut offset = 0i64;
    loop {
        let updates = match app.bot.updates(offset).await {
            Ok(u) => u,
            Err(TgError::RetryAfter(s)) => {
                tokio::time::sleep(Duration::from_secs(s)).await;
                continue;
            }
            Err(e) => {
                eprintln!("getUpdates: {}", e.text());
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
        };
        for u in updates {
            offset = u.update_id + 1;
            if let Some(cb) = u.callback_query {
                let (Some(m), Some(data)) = (cb.message, cb.data) else { continue };
                let chat = m.chat.id;
                if !allowed.is_empty() && !allowed.contains(&chat) {
                    continue;
                }
                println!("chat {chat}: [{data}]");
                let app = app.clone();
                tokio::spawn(async move {
                    // Answer at once so the spinner stops; extra clicks while busy get a toast.
                    if app.begin(chat).await {
                        app.bot.answer(&cb.id, None).await;
                        app.on_button(chat, &data).await;
                        app.end(chat).await;
                    } else {
                        app.bot.answer(&cb.id, Some("Зачекай секунду…")).await;
                    }
                });
                continue;
            }
            let Some(m) = u.message else { continue };
            let chat = m.chat.id;
            if !allowed.is_empty() && !allowed.contains(&chat) {
                println!("ignored chat {chat}");
                continue;
            }
            let text = m.text.unwrap_or_default();
            println!("chat {chat}: {text}");
            let app = app.clone();
            tokio::spawn(async move {
                let msg = m.message_id;
                let t = text.trim().to_string();
                if app.begin(chat).await {
                    app.on_text(chat, msg, &t).await;
                    app.end(chat).await;
                } else {
                    app.bot.delete(chat, msg).await.ok();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_dot() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let html = "<p>Графік погодинних відключень на 20.10.2026</p><p>Група 5.1. Електроенергії немає з 12:00 до 16:00.</p><p>Група 6.1. Електроенергія є.</p>";
        let s = Schedules { today: loe::parse_day(html), tomorrow: None };
        let h = |g: &str| vec![House { name: "Дім".into(), address: String::new(), groups: vec![g.into()] }];
        assert_eq!(dot_at(&s, &h("5.1"), today, 13 * 60), "🔴");
        assert_eq!(dot_at(&s, &h("5.1"), today, 9 * 60), "🟠");
        assert_eq!(dot_at(&s, &h("5.1"), today, 17 * 60), "🟢");
        assert_eq!(dot_at(&s, &h("6.1"), today, 13 * 60), "🟢");
        assert_eq!(dot_at(&Schedules::default(), &h("5.1"), today, 13 * 60), "🟢");
    }
}

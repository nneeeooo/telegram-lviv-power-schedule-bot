//! The menu lives in the card itself (message 1): its caption + inline buttons change per step, the
//! picture stays. Text the user types (street + house, a custom name) is deleted right away, so the
//! chat keeps exactly two messages: the card and the latest info message.
//!
//! Add flow: громада (buttons) -> населений пункт (buttons) -> "вулиця будинок" (typed) ->
//! [street / building / group buttons when ambiguous] -> name (buttons or typed).

use crate::loe::{self, Named};
use crate::state::House;
use crate::tg::{btn, inline};
use crate::{caption, App};
use serde_json::Value;

const PAGE: usize = 15;
const NAMES: [&str; 4] = ["🏠 Дім", "🏢 Робота", "👵 Батьки", "🌲 Дача"];

#[derive(Clone, Debug, Default)]
pub enum Menu {
    #[default]
    Main,
    Houses,
    Otg { page: usize },
    City { otg: Named, page: usize },
    Address { city: Named },
    Street { city: Named, building: String, options: Vec<Named> },
    Building { city: Named, street: Named, similar: Vec<String> },
    Groups,
    PickGroup { label: String, groups: Vec<String> },
    Name { label: String, groups: Vec<String> },
    OwnName { label: String, groups: Vec<String> },
}

#[derive(Clone, Debug, Default)]
pub struct Ui {
    pub menu: Menu,
    /// One-line hint / error shown above the step (e.g. "street not found").
    pub note: Option<String>,
}

type Rows = Vec<Vec<(String, String)>>;

fn back(label: &str, data: &str) -> Vec<(String, String)> {
    vec![btn(label, data)]
}

/// Paged grid of names, `prefix:<index>` callbacks, plus prev/next.
fn paged(items: &[Named], page: usize, prefix: &str, cols: usize) -> Rows {
    let pages = items.len().div_ceil(PAGE).max(1);
    let page = page.min(pages - 1);
    let start = page * PAGE;
    let slice = &items[start..(start + PAGE).min(items.len())];
    let mut rows: Rows = slice
        .chunks(cols)
        .enumerate()
        .map(|(r, ch)| ch.iter().enumerate().map(|(c, n)| btn(n.name.clone(), format!("{prefix}:{}", start + r * cols + c))).collect())
        .collect();
    if pages > 1 {
        let mut nav = vec![];
        if page > 0 {
            nav.push(btn("◀️", format!("{prefix}p:{}", page - 1)));
        }
        nav.push(btn(format!("{}/{}", page + 1, pages), "noop"));
        if page + 1 < pages {
            nav.push(btn("▶️", format!("{prefix}p:{}", page + 1)));
        }
        rows.push(nav);
    }
    rows
}

impl App {
    /// Caption + keyboard of the card for the chat's current menu.
    pub async fn face(&self, chat: i64) -> (String, Value) {
        let ui = self.ui.lock().await.get(&chat).cloned().unwrap_or_default();
        let houses = self.state.lock().await.chats.get(&chat).map(|c| c.houses.clone()).unwrap_or_default();
        let note = ui.note.map(|n| format!("{n}\n\n")).unwrap_or_default();
        let (text, rows): (String, Rows) = match &ui.menu {
            Menu::Main => {
                let s = self.last.lock().await.clone().unwrap_or_default();
                (
                    caption(&s, &houses),
                    vec![
                        vec![btn("➕ Слідкувати за адресою", "add")],
                        vec![btn(format!("📋 Мої адреси ({})", houses.len()), "list")],
                    ],
                )
            }
            Menu::Houses => {
                if houses.is_empty() {
                    ("Адрес ще немає.".into(), vec![vec![btn("➕ Додати", "add"), btn("⬅️ Назад", "main")]])
                } else {
                    let lines: Vec<String> =
                        houses.iter().map(|h| format!("• {} — {} (група {})", h.name, h.address, h.groups.join(", "))).collect();
                    let mut rows: Rows = houses
                        .iter()
                        .enumerate()
                        .map(|(i, h)| vec![btn(format!("🗑 {} · {}", h.name, h.groups.join(", ")), format!("del:{i}"))])
                        .collect();
                    rows.push(vec![btn("➕ Додати", "add"), btn("⬅️ Назад", "main")]);
                    (format!("Мої адреси:\n{}\n\n🗑 — перестати слідкувати.", lines.join("\n")), rows)
                }
            }
            Menu::Otg { page } => {
                let otgs = self.loe.otgs().await.unwrap_or_default();
                let mut rows = paged(&otgs, *page, "o", 3);
                rows.push(vec![btn("🔢 Знаю свою групу", "groups"), btn("✖️ Скасувати", "main")]);
                ("Крок 1/3 · Вибери громаду (ОТГ):".into(), rows)
            }
            Menu::City { otg, page } => {
                let cities = self.loe.cities(otg).await.unwrap_or_default();
                let mut rows = paged(&cities, *page, "c", 3);
                rows.push(back("⬅️ До громад", "add"));
                (format!("Крок 2/3 · {} громада: вибери населений пункт:", otg.name), rows)
            }
            Menu::Address { city } => (
                format!("Крок 3/3 · {}\nНапиши вулицю і номер будинку одним повідомленням, наприклад:\nГородоцька 171", city.name),
                vec![back("✖️ Скасувати", "main")],
            ),
            Menu::Street { city, building, options } => {
                let mut rows: Rows = options
                    .chunks(2)
                    .enumerate()
                    .map(|(r, ch)| ch.iter().enumerate().map(|(c, s)| btn(s.name.clone(), format!("s:{}", r * 2 + c))).collect())
                    .collect();
                rows.push(back("✖️ Скасувати", "main"));
                (format!("{}: яка вулиця? (будинок {building})", city.name), rows)
            }
            Menu::Building { city, street, similar } => {
                let mut rows: Rows = similar
                    .chunks(4)
                    .enumerate()
                    .map(|(r, ch)| ch.iter().enumerate().map(|(c, n)| btn(n.clone(), format!("b:{}", r * 4 + c))).collect())
                    .collect();
                rows.push(back("✖️ Скасувати", "main"));
                (format!("{}, {}: такого номера немає. Може, один з цих? Або напиши ще раз «вулиця номер».", city.name, street.name), rows)
            }
            Menu::Groups => {
                let groups: Vec<String> = (1..=6).flat_map(|q| [format!("{q}.1"), format!("{q}.2")]).collect();
                let mut rows: Rows = groups.chunks(4).map(|ch| ch.iter().map(|g| btn(g.clone(), format!("g:{g}"))).collect()).collect();
                rows.push(back("⬅️ Назад", "add"));
                ("Вибери групу (чергу) ГПВ:".into(), rows)
            }
            Menu::PickGroup { label, groups } => {
                let mut rows: Rows = vec![groups.iter().map(|g| btn(format!("Група {g}"), format!("pg:{g}"))).collect()];
                rows.push(vec![btn("Всі групи", "pg:all")]);
                rows.push(back("✖️ Скасувати", "main"));
                (format!("{label}\nУ будинку кілька груп ({}). Яка твоя? Не знаєш — «Всі групи».", groups.join(", ")), rows)
            }
            Menu::Name { label, groups } => (
                format!("{label} — група {}\nЯк назвати?", groups.join(", ")),
                vec![
                    NAMES.iter().take(2).enumerate().map(|(i, n)| btn(*n, format!("n:{i}"))).collect(),
                    NAMES.iter().skip(2).enumerate().map(|(i, n)| btn(*n, format!("n:{}", i + 2))).collect(),
                    vec![btn("📍 Як адреса", "n:addr"), btn("✏️ Своя назва", "n:own")],
                    back("✖️ Скасувати", "main"),
                ],
            ),
            Menu::OwnName { label, .. } => (format!("{label}\nНапиши назву (до 40 символів)."), vec![back("✖️ Скасувати", "main")]),
        };
        (format!("{note}{text}"), inline(rows))
    }

    async fn set_ui(&self, chat: i64, menu: Menu, note: Option<String>) {
        self.ui.lock().await.insert(chat, Ui { menu, note });
    }

    async fn menu(&self, chat: i64) -> Menu {
        self.ui.lock().await.get(&chat).map(|u| u.menu.clone()).unwrap_or_default()
    }

    /// Switch menu and redraw only caption + buttons.
    async fn go(&self, chat: i64, menu: Menu, note: Option<String>) {
        self.set_ui(chat, menu, note).await;
        self.redraw_caption(chat).await;
    }

    async fn finish(&self, chat: i64, name: String, label: String, groups: Vec<String>) {
        let n = name.clone();
        {
            let mut st = self.state.lock().await;
            st.chats.entry(chat).or_default().houses.push(House { name, address: label, groups });
            st.save().await;
        }
        self.set_ui(chat, Menu::Main, Some(format!("✅ Слідкую за «{n}»"))).await;
        self.sync_chat(chat, true).await;
        // The note is a one-shot: the next redraw shows the plain main screen.
        if let Some(u) = self.ui.lock().await.get_mut(&chat) {
            u.note = None;
        }
    }

    /// Typed text: only the address and own-name steps take input. The message is always deleted.
    pub async fn on_input(&self, chat: i64, text: &str) {
        match self.menu(chat).await {
            Menu::Address { city } | Menu::Building { city, .. } => self.on_address(chat, city, text).await,
            Menu::OwnName { label, groups } => {
                let name: String = text.trim().chars().take(40).collect();
                self.finish(chat, name, label, groups).await;
            }
            _ => {}
        }
    }

    async fn on_address(&self, chat: i64, city: Named, text: &str) {
        let Some((street_q, building)) = loe::split_address(text) else {
            let note = "Не бачу номера будинку. Напиши, наприклад: Городоцька 171".to_string();
            return self.go(chat, Menu::Address { city }, Some(note)).await;
        };
        let streets = match self.loe.streets(city.id).await {
            Ok(s) => s,
            Err(e) => return self.go(chat, Menu::Address { city }, Some(format!("⚠️ {e}"))).await,
        };
        let hits: Vec<Named> = loe::search(&streets, &street_q, 10).into_iter().cloned().collect();
        let exact = hits.first().filter(|s| s.name.to_lowercase() == street_q.to_lowercase()).cloned();
        match (exact, hits.len()) {
            (_, 0) => self.go(chat, Menu::Address { city }, Some(format!("Вулицю «{street_q}» не знайдено."))).await,
            (Some(street), _) => self.on_street(chat, city, street, &building).await,
            (None, 1) => self.on_street(chat, city, hits[0].clone(), &building).await,
            (None, _) => self.go(chat, Menu::Street { city, building, options: hits }, None).await,
        }
    }

    async fn on_street(&self, chat: i64, city: Named, street: Named, building: &str) {
        let idx = match self.loe.houses(city.id, street.id).await {
            Ok(i) => i,
            Err(e) => return self.go(chat, Menu::Address { city }, Some(format!("⚠️ {e}"))).await,
        };
        match idx.get(building) {
            Some((bname, groups)) => {
                let label = format!("{}, {} {}", city.name, street.name, bname);
                if groups.len() == 1 {
                    self.go(chat, Menu::Name { label, groups: groups.clone() }, None).await;
                } else {
                    self.go(chat, Menu::PickGroup { label, groups: groups.clone() }, None).await;
                }
            }
            None => {
                let similar = idx.similar(building, 12);
                if similar.is_empty() {
                    let note = format!("На вул. {} будинку «{building}» немає в графіку. Перевір номер.", street.name);
                    self.go(chat, Menu::Address { city }, Some(note)).await;
                } else {
                    self.go(chat, Menu::Building { city, street, similar }, None).await;
                }
            }
        }
    }

    /// Button presses on the card.
    pub async fn on_button(&self, chat: i64, data: &str) {
        let (kind, val) = data.split_once(':').unwrap_or((data, ""));
        let i: usize = val.parse().unwrap_or(usize::MAX);
        let menu = self.menu(chat).await;
        match (kind, menu) {
            ("noop", _) => {}
            ("main", _) => self.go(chat, Menu::Main, None).await,
            ("refresh", _) => {
                if let Err(e) = self.fetch().await {
                    self.go(chat, Menu::Main, Some(format!("⚠️ {e}"))).await;
                } else {
                    self.sync_chat(chat, true).await;
                }
            }
            ("list", _) => self.go(chat, Menu::Houses, None).await,
            ("add", _) => self.go(chat, Menu::Otg { page: 0 }, None).await,
            ("groups", _) => self.go(chat, Menu::Groups, None).await,
            ("del", _) => {
                let removed = {
                    let mut st = self.state.lock().await;
                    let c = st.chats.entry(chat).or_default();
                    let r = (i < c.houses.len()).then(|| c.houses.remove(i));
                    st.save().await;
                    r
                };
                self.set_ui(chat, Menu::Houses, removed.map(|h| format!("Більше не слідкую за «{}»", h.name))).await;
                self.sync_chat(chat, true).await;
            }
            ("op", _) => self.go(chat, Menu::Otg { page: i }, None).await,
            ("o", _) => {
                let otgs = self.loe.otgs().await.unwrap_or_default();
                match otgs.get(i) {
                    Some(otg) => self.go(chat, Menu::City { otg: otg.clone(), page: 0 }, None).await,
                    None => self.go(chat, Menu::Otg { page: 0 }, None).await,
                }
            }
            ("cp", Menu::City { otg, .. }) => self.go(chat, Menu::City { otg, page: i }, None).await,
            ("c", Menu::City { otg, .. }) => {
                let cities = self.loe.cities(&otg).await.unwrap_or_default();
                match cities.get(i) {
                    Some(city) => self.go(chat, Menu::Address { city: city.clone() }, None).await,
                    None => self.go(chat, Menu::City { otg, page: 0 }, None).await,
                }
            }
            ("s", Menu::Street { city, building, options }) => match options.get(i) {
                Some(street) => self.on_street(chat, city, street.clone(), &building).await,
                None => self.go(chat, Menu::Address { city }, None).await,
            },
            ("b", Menu::Building { city, street, similar }) => match similar.get(i) {
                Some(b) => self.on_street(chat, city, street, &b.clone()).await,
                None => self.go(chat, Menu::Address { city }, None).await,
            },
            ("g", _) if loe::valid_group(val) => {
                self.go(chat, Menu::Name { label: format!("Група {val}"), groups: vec![val.to_string()] }, None).await
            }
            ("pg", Menu::PickGroup { label, groups }) => {
                let chosen = if val == "all" { groups } else { vec![val.to_string()] };
                self.go(chat, Menu::Name { label, groups: chosen }, None).await;
            }
            ("n", Menu::Name { label, groups }) => match val {
                "own" => self.go(chat, Menu::OwnName { label, groups }, None).await,
                "addr" => {
                    let name = label.split_once(", ").map(|(_, r)| r.to_string()).unwrap_or_else(|| label.clone());
                    self.finish(chat, name, label, groups).await;
                }
                _ => {
                    let name = NAMES.get(i).and_then(|n| n.split_once(' ')).map(|(_, t)| t.to_string()).unwrap_or_else(|| "Дім".into());
                    self.finish(chat, name, label, groups).await;
                }
            },
            // A button from a step the user already left: just show the current screen again.
            _ => self.redraw_caption(chat).await,
        }
    }
}

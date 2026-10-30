//! Lvivoblenergo (LOE) data, the same endpoints poweron.loe.lviv.ua uses (unofficial, undocumented):
//! - schedules: api.loe.lviv.ua/api/menus?type=photo-grafic -> items "Today" / "Tomorrow" with
//!   rawHtml ("Група 1.1. Електроенергії немає з 20:00 до 22:00.") and the official PNG;
//! - address -> group (черга ГПВ): power-api.loe.lviv.ua/api/pw_cities / pw_streets / pw_accounts.

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const SCHEDULE: &str = "https://api.loe.lviv.ua/api/menus?page=1&type=photo-grafic";
const POWER: &str = "https://power-api.loe.lviv.ua/api";

/// Minutes from midnight, end exclusive; "24:00" = 1440.
pub type Window = (u32, u32);

#[derive(Debug, Clone, PartialEq)]
pub struct DaySchedule {
    pub date: Option<NaiveDate>,
    /// "18:56 17.10.2026" as LOE writes it.
    pub as_of: String,
    /// "1.1" -> outage windows (empty = power all day).
    pub groups: BTreeMap<String, Vec<Window>>,
    /// Raw text, used to detect changes.
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Schedules {
    pub today: Option<DaySchedule>,
    pub tomorrow: Option<DaySchedule>,
}

impl Schedules {
    pub fn is_empty(&self) -> bool {
        self.today.is_none() && self.tomorrow.is_none()
    }
    pub fn days(&self) -> impl Iterator<Item = &DaySchedule> {
        self.today.iter().chain(self.tomorrow.iter())
    }
    /// Stable fingerprint of what LOE published.
    pub fn fingerprint(&self) -> String {
        self.days().map(|d| d.raw.as_str()).collect::<Vec<_>>().join("\n--\n")
    }
    pub fn as_of(&self) -> Option<&str> {
        self.days().map(|d| d.as_of.as_str()).filter(|s| !s.is_empty()).last()
    }
}

fn re(pat: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("regex"))
}

fn minutes(hm: &str) -> Option<u32> {
    let (h, m) = hm.split_once(':')?;
    Some(h.trim().parse::<u32>().ok()? * 60 + m.trim().parse::<u32>().ok()?)
}

pub fn parse_day(html: &str) -> Option<DaySchedule> {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static DATE: OnceLock<Regex> = OnceLock::new();
    static ASOF: OnceLock<Regex> = OnceLock::new();
    static GROUP: OnceLock<Regex> = OnceLock::new();
    static WIN: OnceLock<Regex> = OnceLock::new();
    let text = re(r"<[^>]+>", &TAGS).replace_all(html, "\n");
    let text = text.replace("&nbsp;", " ");
    if !text.contains("Група") {
        return None;
    }
    let date = re(r"відключень на (\d{2})\.(\d{2})\.(\d{4})", &DATE).captures(&text).and_then(|c| {
        NaiveDate::from_ymd_opt(c[3].parse().ok()?, c[2].parse().ok()?, c[1].parse().ok()?)
    });
    let as_of = re(r"станом на ([0-9:]+ [0-9.]+)", &ASOF).captures(&text).map(|c| c[1].to_string()).unwrap_or_default();
    let mut groups = BTreeMap::new();
    for line in text.lines() {
        let Some(c) = re(r"Група\s+(\d+\.\d+)", &GROUP).captures(line) else { continue };
        let wins: Vec<Window> = re(r"з\s*(\d{1,2}:\d{2})\s*до\s*(\d{1,2}:\d{2})", &WIN)
            .captures_iter(line)
            .filter_map(|w| Some((minutes(&w[1])?, minutes(&w[2])?)))
            .collect();
        groups.insert(c[1].to_string(), wins);
    }
    Some(DaySchedule { date, as_of, groups, raw: text.trim().to_string() })
}

/// The photo-grafic menu items; LOE returns a plain array or a hydra collection depending on Accept.
async fn menu_items(http: &reqwest::Client) -> Result<Vec<Value>> {
    let v: Value = http.get(SCHEDULE).send().await?.error_for_status()?.json().await?;
    let menu = members(v).into_iter().next().context("no photo-grafic menu")?;
    Ok(menu["menuItems"].as_array().cloned().context("menuItems")?)
}

pub async fn fetch_schedules(http: &reqwest::Client) -> Result<Schedules> {
    let items = menu_items(http).await?;
    let day = |name: &str| {
        items
            .iter()
            .find(|i| i["name"] == name)
            .and_then(|i| i["rawHtml"].as_str())
            .and_then(parse_day)
    };
    Ok(Schedules { today: day("Today"), tomorrow: day("Tomorrow") })
}

/// Latest archived schedule as "today", for local rendering tests when nothing is published.
pub async fn fetch_archive_sample(http: &reqwest::Client) -> Result<Schedules> {
    let items = menu_items(http).await?;
    let arch = items.iter().find(|i| i["name"] == "Arhiv").and_then(|i| i["children"].as_array()).context("Arhiv")?;
    let today = arch.iter().rev().find_map(|c| c["rawHtml"].as_str().and_then(parse_day));
    Ok(Schedules { today, tomorrow: None })
}

/// API Platform answers either a plain array or {"hydra:member": [...]}.
fn members(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        mut o => o["hydra:member"].take().as_array().cloned().unwrap_or_default(),
    }
}

fn norm(s: &str) -> String {
    let s = s.to_lowercase().replace('’', "'").replace('ʼ', "'");
    let s = s.trim_start_matches("вул.").trim_start_matches("вулиця").trim();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug)]
pub struct Named {
    pub id: i64,
    pub name: String,
}

fn named(items: &[Value]) -> Vec<Named> {
    items
        .iter()
        .filter_map(|v| Some(Named { id: v["id"].as_i64()?, name: v["name"].as_str()?.trim().to_string() }))
        .filter(|n| !n.name.is_empty() && n.name != "невідомо")
        .collect()
}

/// Ukrainian alphabetical sort key (byte order puts "І" before "Б").
pub fn uk_key(s: &str) -> Vec<u32> {
    const ABC: &str = "абвгґдеєжзиіїйклмнопрстуфхцчшщьюя";
    s.to_lowercase()
        .chars()
        .map(|c| ABC.chars().position(|a| a == c).map(|p| p as u32 + 1000).unwrap_or(c as u32 + 2000))
        .collect()
}

/// Sort by Ukrainian alphabet and number repeated names: "Вулька", "Вулька (2)".
fn sort_dedupe(items: &mut Vec<Named>, first: impl Fn(&Named) -> bool) {
    items.sort_by(|a, b| first(b).cmp(&first(a)).then(uk_key(&a.name).cmp(&uk_key(&b.name))));
    let mut seen: HashMap<String, usize> = HashMap::new();
    for n in items.iter_mut() {
        let k = seen.entry(n.name.clone()).or_insert(0);
        *k += 1;
        if *k > 1 {
            n.name = format!("{} ({k})", n.name);
        }
    }
}

/// Matches for a typed query: exact (normalised) names first, then prefix, then "contains".
pub fn search<'a>(items: &'a [Named], query: &str, limit: usize) -> Vec<&'a Named> {
    let q = norm(query);
    if q.is_empty() {
        return vec![];
    }
    let mut exact: Vec<&Named> = items.iter().filter(|n| norm(&n.name) == q).collect();
    let mut rest: Vec<&Named> = items.iter().filter(|n| norm(&n.name) != q && norm(&n.name).contains(&q)).collect();
    rest.sort_by(|a, b| {
        let (sa, sb) = (norm(&a.name).starts_with(&q), norm(&b.name).starts_with(&q));
        sb.cmp(&sa).then(a.name.cmp(&b.name))
    });
    exact.extend(rest);
    exact.truncate(limit);
    exact
}

/// "204Д", "204-д", "204 Д" -> "204д": LOE writes letters after a hyphen ("204-Д").
pub fn norm_building(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| !matches!(c, ' ' | '-' | '.' | ',')).collect()
}

fn norm_group(g: &str) -> String {
    if g.len() == 2 && !g.contains('.') { format!("{}.{}", &g[..1], &g[1..]) } else { g.to_string() }
}

/// "Городоцька 171", "Шевченка Т., 1", "Зелена 204 Д" -> ("Городоцька", "171") etc.
pub fn split_address(s: &str) -> Option<(String, String)> {
    let s = s.replace(',', " ");
    let words: Vec<&str> = s.split_whitespace().collect();
    let idx = words.iter().rposition(|w| w.chars().any(|c| c.is_ascii_digit()))?;
    // "Зелена 204 Д": a lone letter after the number belongs to the number.
    let street = words[..idx].join(" ");
    let building = words[idx..].join(" ");
    (!street.is_empty()).then_some((street, building))
}

/// Buildings of one street: normalised number -> (LOE spelling, groups).
pub struct StreetIndex {
    pub buildings: BTreeMap<String, (String, Vec<String>)>,
    at: Instant,
}

impl StreetIndex {
    pub fn get(&self, b: &str) -> Option<&(String, Vec<String>)> {
        self.buildings.get(&norm_building(b)).filter(|(_, g)| !g.is_empty())
    }
    /// Same house number with other letters / blocks ("204" -> "204-А", "204-Д"), for buttons.
    pub fn similar(&self, b: &str, limit: usize) -> Vec<String> {
        let digits: String = b.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return vec![];
        }
        self.buildings
            .values()
            .filter(|(n, g)| !g.is_empty() && n.starts_with(&digits) && !n[digits.len()..].starts_with(|c: char| c.is_ascii_digit()))
            .map(|(n, _)| n.clone())
            .take(limit)
            .collect()
    }
}

/// LOE address API with a request gate (one at a time, >= GAP apart, retries on 403/429/5xx:
/// fast clicking used to trip their firewall) and caches, so each lookup costs few requests.
pub struct Loe {
    http: reqwest::Client,
    gate: Mutex<Option<Instant>>,
    otgs: Mutex<Option<Vec<Named>>>,
    cities: Mutex<HashMap<i64, Vec<Named>>>,
    streets: Mutex<HashMap<i64, Vec<Named>>>,
    houses: Mutex<HashMap<i64, Arc<StreetIndex>>>,
}

const GAP: Duration = Duration::from_millis(1200);
const HOUSES_TTL: Duration = Duration::from_secs(6 * 3600);

impl Loe {
    pub fn new(http: reqwest::Client) -> Self {
        Loe {
            http,
            gate: Mutex::new(None),
            otgs: Mutex::new(None),
            cities: Mutex::new(HashMap::new()),
            streets: Mutex::new(HashMap::new()),
            houses: Mutex::new(HashMap::new()),
        }
    }

    async fn get(&self, path: &str) -> Result<Vec<Value>> {
        let mut last = self.gate.lock().await; // held for the whole request: strictly serial
        let mut delay = Duration::from_secs(3);
        for attempt in 0..4 {
            if let Some(t) = *last {
                let wait = GAP.saturating_sub(t.elapsed());
                tokio::time::sleep(wait).await;
            }
            let resp = self.http.get(format!("{POWER}/{path}")).send().await;
            *last = Some(Instant::now());
            match resp {
                Ok(r) if r.status().is_success() => return Ok(members(r.json().await?)),
                Ok(r) if attempt < 3 && (r.status().as_u16() == 403 || r.status().as_u16() == 429 || r.status().is_server_error()) => {
                    eprintln!("LOE {} on {path}, retry in {delay:?}", r.status());
                }
                Ok(r) => bail!("Львівобленерго відповіло {}", r.status()),
                Err(e) if attempt < 3 => eprintln!("LOE {e}, retry in {delay:?}"),
                Err(e) => return Err(e.into()),
            }
            tokio::time::sleep(delay).await;
            delay *= 2;
        }
        bail!("Львівобленерго не відповідає, спробуй за хвилину")
    }

    /// Hromadas (ОТГ), Lviv first.
    pub async fn otgs(&self) -> Result<Vec<Named>> {
        if let Some(o) = self.otgs.lock().await.clone() {
            return Ok(o);
        }
        let mut o = named(&self.get("pw_otgs?pagination=false").await?);
        sort_dedupe(&mut o, |n| n.name == "Львівська");
        *self.otgs.lock().await = Some(o.clone());
        Ok(o)
    }

    /// Settlements of a hromada, its centre first ("Львівська" -> "Львів").
    pub async fn cities(&self, otg: &Named) -> Result<Vec<Named>> {
        if let Some(c) = self.cities.lock().await.get(&otg.id) {
            return Ok(c.clone());
        }
        let mut c = named(&self.get(&format!("pw_cities?pagination=false&otg.id={}", otg.id)).await?);
        // The hromada's centre first ("Львівська" -> "Львів"): same first four letters.
        let stem: String = otg.name.chars().take(4).collect::<String>().to_lowercase();
        sort_dedupe(&mut c, |n| n.name.to_lowercase().starts_with(&stem));
        self.cities.lock().await.insert(otg.id, c.clone());
        Ok(c)
    }

    pub async fn streets(&self, city: i64) -> Result<Vec<Named>> {
        if let Some(s) = self.streets.lock().await.get(&city) {
            return Ok(s.clone());
        }
        let s = named(&self.get(&format!("pw_streets?pagination=false&city.id={city}")).await?);
        self.streets.lock().await.insert(city, s.clone());
        Ok(s)
    }

    /// All buildings of a street in ONE request, cached.
    pub async fn houses(&self, city: i64, street: i64) -> Result<Arc<StreetIndex>> {
        if let Some(h) = self.houses.lock().await.get(&street).filter(|h| h.at.elapsed() < HOUSES_TTL) {
            return Ok(h.clone());
        }
        let acc = self.get(&format!("pw_accounts?pagination=false&city.id={city}&street.id={street}")).await?;
        let mut buildings: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
        for a in &acc {
            let Some(n) = a["buildingName"].as_str().map(str::trim).filter(|n| !n.is_empty()) else { continue };
            let e = buildings.entry(norm_building(n)).or_insert_with(|| (n.to_string(), BTreeSet::new()));
            if let Some(g) = a["chergGpv"].as_str().map(norm_group).filter(|g| valid_group(g)) {
                e.1.insert(g);
            }
        }
        let idx = Arc::new(StreetIndex {
            buildings: buildings.into_iter().map(|(k, (n, g))| (k, (n, g.into_iter().collect()))).collect(),
            at: Instant::now(),
        });
        self.houses.lock().await.insert(street, idx.clone());
        Ok(idx)
    }

    /// CLI helper: "Львів", "Городоцька", "171" -> (label, groups).
    pub async fn resolve(&self, city: &str, street: &str, building: &str) -> Result<(String, Vec<String>)> {
        let cities = named(&self.get("pw_cities?pagination=false").await?);
        let c = search(&cities, city, 1).first().map(|c| (*c).clone()).with_context(|| format!("«{city}» не знайдено"))?;
        let streets = self.streets(c.id).await?;
        let s = search(&streets, street, 1).first().map(|s| (*s).clone()).with_context(|| format!("вулицю «{street}» не знайдено"))?;
        let idx = self.houses(c.id, s.id).await?;
        let (bname, groups) = idx.get(building).cloned().with_context(|| {
            format!("будинок {building} не знайдено; схожі: {}", idx.similar(building, 12).join(", "))
        })?;
        Ok((format!("{}, {} {}", c.name, s.name, bname), groups))
    }
}

pub fn valid_group(g: &str) -> bool {
    static G: OnceLock<Regex> = OnceLock::new();
    re(r"^[1-6]\.[12]$", &G).is_match(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_prefers_exact_then_prefix() {
        let items: Vec<Named> = ["Городоцька-Величка", "Городоцька", "Над Городом", "Шевченка Т."]
            .iter()
            .enumerate()
            .map(|(i, n)| Named { id: i as i64, name: n.to_string() })
            .collect();
        let names: Vec<&str> = search(&items, "городоцька", 5).iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Городоцька", "Городоцька-Величка"]);
        let names: Vec<&str> = search(&items, "город", 5).iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Городоцька", "Городоцька-Величка", "Над Городом"]);
        assert_eq!(search(&items, "вул. Шевченка т.", 5)[0].name, "Шевченка Т.");
    }

    #[test]
    fn building_spellings() {
        assert_eq!(norm_building("204-Д"), norm_building("204д"));
        assert_eq!(norm_building("204 Д"), "204д");
        assert_eq!(split_address("Городоцька 171"), Some(("Городоцька".into(), "171".into())));
        assert_eq!(split_address("Шевченка Т., 1"), Some(("Шевченка Т.".into(), "1".into())));
        assert_eq!(split_address("Зелена 204 Д").map(|x| norm_building(&x.1)), Some("204д".into()));
        assert_eq!(split_address("171"), None);
    }

    #[test]
    fn parses_loe_html() {
        let html = "<div><p><b>Графік погодинних відключень на 17.10.2026</b></p>\n<p><b>Інформація станом на 18:56 17.10.2026</b></p>\n<p>Група 1.1. Електроенергії немає з 20:00 до 22:00.</p>\n<p>Група 1.2. Електроенергії немає з 08:00 до 10:00, з 14:00 до 24:00.</p>\n<p>Група 6.2. Електроенергія є.</p>\n</div>";
        let d = parse_day(html).unwrap();
        assert_eq!(d.date, NaiveDate::from_ymd_opt(2026, 10, 17));
        assert_eq!(d.as_of, "18:56 17.10.2026");
        assert_eq!(d.groups["1.1"], vec![(1200, 1320)]);
        assert_eq!(d.groups["1.2"], vec![(480, 600), (840, 1440)]);
        assert!(d.groups["6.2"].is_empty());
    }
}

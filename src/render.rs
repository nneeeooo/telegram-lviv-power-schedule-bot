//! Cards: the schedule for tracked houses (24 h bars for today / tomorrow) and the party frog
//! for "no schedule".

use crate::loe::{DaySchedule, Schedules, Window};
use crate::state::House;
use crate::text::Text;
use chrono::NaiveDate;
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

const W: u32 = 1400;
const PAD: f32 = 48.0;

fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

fn rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, p: &Paint) {
    if let Some(r) = Rect::from_xywh(x, y, w.max(0.5), h.max(0.5)) {
        pm.fill_rect(r, p, Transform::identity(), None);
    }
}

fn round_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, p: &Paint) {
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, p, FillRule::Winding, Transform::identity(), None);
    }
}

pub fn hm(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

pub fn windows_text(w: &[Window]) -> String {
    if w.is_empty() {
        return "світло є весь день".into();
    }
    let parts: Vec<String> = w.iter().map(|&(a, b)| format!("{}–{}", hm(a), hm(b))).collect();
    format!("немає світла {}", parts.join(", "))
}

fn day_label(d: &DaySchedule, today: NaiveDate) -> String {
    match d.date {
        Some(x) if x == today => format!("Сьогодні {}", x.format("%d.%m")),
        Some(x) if Some(x) == today.succ_opt() => format!("Завтра {}", x.format("%d.%m")),
        Some(x) => x.format("%d.%m").to_string(),
        None => "Графік".into(),
    }
}

pub struct Renderer {
    pub text: Text,
}

impl Renderer {
    /// One block per (house, group): name, then a 24 h bar per published day.
    pub fn schedule(&self, s: &Schedules, houses: &[House], today: NaiveDate, now_label: &str) -> Vec<u8> {
        let rows: Vec<(&House, &String)> = houses.iter().flat_map(|h| h.groups.iter().map(move |g| (h, g))).collect();
        let days: Vec<&DaySchedule> = s.days().collect();
        let header = 190.0;
        let block = 92.0 + days.len() as f32 * 132.0;
        let empty_h = if rows.is_empty() { 200.0 } else { 0.0 };
        let footer = 90.0;
        let h = (header + rows.len() as f32 * block + empty_h + footer).max(W as f32 * 0.62) as u32;
        let mut pm = Pixmap::new(W, h).expect("pixmap");
        pm.fill(Color::from_rgba8(17, 19, 26, 255));

        // Header.
        rect(&mut pm, 0.0, 0.0, W as f32, header - 20.0, &paint(255, 196, 0, 255));
        self.text.draw(&mut pm, "Графік відключень світла", PAD, 86.0, 64.0, (20, 20, 20, 255));
        let sub = match s.as_of() {
            Some(a) => format!("Львівобленерго · станом на {a}"),
            None => "Львівобленерго".into(),
        };
        self.text.draw(&mut pm, &sub, PAD, 140.0, 36.0, (40, 40, 40, 255));

        let mut y = header + 10.0;
        if rows.is_empty() {
            self.text.draw(&mut pm, "Адрес ще немає.", PAD, y + 60.0, 40.0, (240, 240, 245, 255));
            self.text.draw(&mut pm, "Натисни «Слідкувати за адресою» під картинкою", PAD, y + 120.0, 40.0, (255, 196, 0, 255));
            y += empty_h;
        }
        let bar_x = PAD;
        let bar_w = W as f32 - 2.0 * PAD;
        for (house, group) in rows {
            let multi = house.groups.len() > 1;
            let title = if multi {
                format!("{} · група {} (одна з {})", house.name, group, house.groups.len())
            } else if house.name.contains(group.as_str()) {
                house.name.clone()
            } else {
                format!("{} · група {}", house.name, group)
            };
            self.text.draw(&mut pm, &title, PAD, y + 56.0, 44.0, (255, 255, 255, 255));
            y += 82.0;
            for d in &days {
                let wins = d.groups.get(group.as_str()).cloned();
                let label = day_label(d, today);
                self.text.draw(&mut pm, &label, PAD, y + 30.0, 30.0, (190, 192, 200, 255));
                let (desc, col) = match &wins {
                    None => ("групи немає в графіку".to_string(), (190, 192, 200, 255)),
                    Some(w) if w.is_empty() => (windows_text(w), (90, 220, 120, 255)),
                    Some(w) => (windows_text(w), (255, 120, 110, 255)),
                };
                let lw = self.text.width(&label, 30.0);
                self.text.draw(&mut pm, &format!("  {desc}"), PAD + lw, y + 30.0, 30.0, col);
                // 24 h bar: green = power, red = outage.
                let by = y + 46.0;
                let bh = 46.0;
                round_rect(&mut pm, bar_x, by, bar_w, bh, 10.0, &paint(46, 160, 90, 255));
                for &(a, b) in wins.as_deref().unwrap_or(&[]) {
                    let x0 = bar_x + bar_w * a as f32 / 1440.0;
                    let x1 = bar_x + bar_w * b.min(1440) as f32 / 1440.0;
                    rect(&mut pm, x0, by, x1 - x0, bh, &paint(230, 57, 70, 255));
                }
                if wins.is_none() {
                    round_rect(&mut pm, bar_x, by, bar_w, bh, 10.0, &paint(70, 72, 82, 255));
                }
                for hour in (0..=24).step_by(3) {
                    let x = bar_x + bar_w * hour as f32 / 24.0;
                    rect(&mut pm, x - 1.0, by + bh, 2.0, 8.0, &paint(150, 152, 160, 255));
                    let t = format!("{hour}");
                    let tw = self.text.width(&t, 22.0);
                    let tx = (x - tw / 2.0).clamp(bar_x, bar_x + bar_w - tw);
                    self.text.draw(&mut pm, &t, tx, by + bh + 32.0, 22.0, (150, 152, 160, 255));
                }
                y += 132.0;
            }
            y += 10.0;
            rect(&mut pm, PAD, y - 6.0, bar_w, 2.0, &paint(255, 255, 255, 30));
        }
        let foot = format!("Оновлено {now_label} · джерело poweron.loe.lviv.ua");
        self.text.draw(&mut pm, &foot, PAD, h as f32 - 36.0, 26.0, (150, 152, 160, 255));
        pm.encode_png().expect("png")
    }

    /// "No schedule" card: a celebrating frog with a party hat and confetti.
    pub fn frog(&self, now_label: &str) -> Vec<u8> {
        let (w, h) = (1200u32, 1200u32);
        let mut pm = Pixmap::new(w, h).expect("pixmap");
        pm.fill(Color::from_rgba8(255, 236, 160, 255));
        let id = Transform::identity();
        // Sun rays.
        for i in 0..16 {
            let a = i as f32 * std::f32::consts::PI / 8.0;
            let mut pb = PathBuilder::new();
            pb.move_to(600.0, 640.0);
            pb.line_to(600.0 + 1200.0 * (a - 0.09).cos(), 640.0 + 1200.0 * (a - 0.09).sin());
            pb.line_to(600.0 + 1200.0 * (a + 0.09).cos(), 640.0 + 1200.0 * (a + 0.09).sin());
            pb.close();
            if let Some(p) = pb.finish() {
                pm.fill_path(&p, &paint(255, 214, 90, 255), FillRule::Winding, id, None);
            }
        }
        // Confetti (deterministic pseudo-random).
        let colors = [(230, 57, 70), (52, 152, 219), (46, 204, 113), (155, 89, 182), (255, 140, 0), (241, 196, 15)];
        let mut seed: u32 = 7;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as f32 / 16_777_216.0
        };
        for i in 0..90 {
            let (x, y) = (rnd() * w as f32, rnd() * h as f32 * 0.95);
            let (r, g, b) = colors[i % colors.len()];
            let tr = Transform::from_rotate(rnd() * 180.0).post_translate(x, y);
            if let Some(r_) = Rect::from_xywh(-9.0, -4.0, 18.0, 8.0) {
                pm.fill_rect(r_, &paint(r, g, b, 255), tr, None);
            }
        }
        let green = paint(84, 186, 72, 255);
        let dark = paint(38, 110, 40, 255);
        let circle = |pm: &mut Pixmap, x: f32, y: f32, r: f32, p: &Paint| {
            if let Some(c) = PathBuilder::from_circle(x, y, r) {
                pm.fill_path(&c, p, FillRule::Winding, Transform::identity(), None);
            }
        };
        let oval = |pm: &mut Pixmap, x: f32, y: f32, rx: f32, ry: f32, p: &Paint| {
            if let Some(r) = Rect::from_xywh(x - rx, y - ry, 2.0 * rx, 2.0 * ry) {
                if let Some(path) = PathBuilder::from_oval(r) {
                    pm.fill_path(&path, p, FillRule::Winding, Transform::identity(), None);
                }
            }
        };
        // Raised arms (behind the body).
        let arm = |pm: &mut Pixmap, sx: f32| {
            let mut pb = PathBuilder::new();
            pb.move_to(600.0 + sx * 190.0, 780.0);
            pb.quad_to(600.0 + sx * 330.0, 700.0, 600.0 + sx * 360.0, 520.0);
            let s = Stroke { width: 46.0, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &paint(84, 186, 72, 255), &s, Transform::identity(), None);
            }
        };
        arm(&mut pm, -1.0);
        arm(&mut pm, 1.0);
        for sx in [-1.0f32, 1.0] {
            for k in 0..3 {
                let a = -1.9 + k as f32 * 0.45;
                circle(&mut pm, 600.0 + sx * 360.0 + sx * 34.0 * a.cos(), 520.0 + 34.0 * a.sin(), 16.0, &green);
            }
        }
        // Body, belly, head.
        oval(&mut pm, 600.0, 860.0, 250.0, 200.0, &dark);
        oval(&mut pm, 600.0, 850.0, 240.0, 190.0, &green);
        oval(&mut pm, 600.0, 900.0, 150.0, 120.0, &paint(196, 236, 160, 255));
        oval(&mut pm, 600.0, 640.0, 270.0, 175.0, &dark);
        oval(&mut pm, 600.0, 630.0, 262.0, 168.0, &green);
        // Eyes.
        for sx in [-1.0f32, 1.0] {
            let ex = 600.0 + sx * 135.0;
            circle(&mut pm, ex, 500.0, 92.0, &dark);
            circle(&mut pm, ex, 495.0, 86.0, &green);
            circle(&mut pm, ex, 495.0, 60.0, &paint(255, 255, 255, 255));
            circle(&mut pm, ex + sx * 8.0, 500.0, 32.0, &paint(25, 25, 25, 255));
            circle(&mut pm, ex + sx * 8.0 + 11.0, 488.0, 11.0, &paint(255, 255, 255, 255));
        }
        // Cheeks + big open smile.
        oval(&mut pm, 430.0, 680.0, 44.0, 26.0, &paint(255, 130, 150, 200));
        oval(&mut pm, 770.0, 680.0, 44.0, 26.0, &paint(255, 130, 150, 200));
        let mut pb = PathBuilder::new();
        pb.move_to(470.0, 650.0);
        pb.quad_to(600.0, 820.0, 730.0, 650.0);
        pb.quad_to(600.0, 700.0, 470.0, 650.0);
        pb.close();
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, &paint(120, 30, 40, 255), FillRule::Winding, id, None);
        }
        oval(&mut pm, 600.0, 735.0, 55.0, 26.0, &paint(255, 110, 130, 255));
        // Party hat with stripes and a pompom.
        let mut pb = PathBuilder::new();
        pb.move_to(520.0, 470.0);
        pb.line_to(690.0, 470.0);
        pb.line_to(640.0, 230.0);
        pb.close();
        if let Some(p) = pb.finish() {
            let hat_t = Transform::from_rotate_at(12.0, 605.0, 470.0);
            pm.fill_path(&p, &paint(155, 89, 182, 255), FillRule::Winding, hat_t, None);
            for k in 0..3 {
                let yy = 420.0 - k as f32 * 70.0;
                let half = 85.0 * (yy - 230.0) / 240.0;
                let cx = 605.0 + 35.0 * (470.0 - yy) / 240.0;
                if let Some(r) = Rect::from_xywh(cx - half, yy - 12.0, 2.0 * half, 24.0) {
                    let mut mask = tiny_skia::Mask::new(w, h).unwrap();
                    mask.fill_path(&p, FillRule::Winding, true, hat_t);
                    pm.fill_rect(r, &paint(255, 214, 10, 255), hat_t, Some(&mask));
                }
            }
            let mut tip = tiny_skia::Point::from_xy(640.0, 230.0);
            hat_t.map_point(&mut tip);
            circle(&mut pm, tip.x, tip.y, 26.0, &paint(230, 57, 70, 255));
        }
        // Text.
        let title = "Графіка немає!";
        let tw = self.text.width(title, 104.0);
        round_rect(&mut pm, 600.0 - tw / 2.0 - 36.0, 60.0, tw + 72.0, 140.0, 30.0, &paint(255, 255, 255, 235));
        self.text.draw(&mut pm, title, 600.0 - tw / 2.0, 165.0, 104.0, (30, 120, 40, 255));
        let sub = "Світло має бути весь день";
        let sw = self.text.width(sub, 46.0);
        round_rect(&mut pm, 600.0 - sw / 2.0 - 24.0, 1080.0, sw + 48.0, 74.0, 22.0, &paint(255, 255, 255, 220));
        self.text.draw(&mut pm, sub, 600.0 - sw / 2.0, 1132.0, 46.0, (40, 40, 40, 255));
        let foot = format!("перевірено {now_label}");
        let fw = self.text.width(&foot, 26.0);
        self.text.draw(&mut pm, &foot, 600.0 - fw / 2.0, 1186.0, 26.0, (110, 90, 40, 255));
        pm.encode_png().expect("png")
    }
}

use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScheduleAction {
    Rec,
    Play,
}

#[derive(Debug, Clone, Copy)]
pub enum ScheduleMsg {
    Start(ScheduleAction),
    Stop(ScheduleAction),
}

// ---------------------------------------------------------------------------
// Schedule entry
// ---------------------------------------------------------------------------

pub struct ScheduleEntry {
    pub cron: CronExpr,
    pub action: ScheduleAction,
    pub duration_secs: u64,
}

impl ScheduleEntry {
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        let cron = CronExpr::parse(v["cron"].as_str()?)?;
        let action = match v["action"].as_str()? {
            "rec" => ScheduleAction::Rec,
            "play" => ScheduleAction::Play,
            _ => return None,
        };
        let duration_secs = if let Some(m) = v["duration_minutes"].as_u64() {
            m * 60
        } else if let Some(s) = v["duration_seconds"].as_u64() {
            s
        } else {
            return None;
        };
        Some(ScheduleEntry { cron, action, duration_secs })
    }
}

// ---------------------------------------------------------------------------
// Load from ~/.config/octotrack/schedules.json
// ---------------------------------------------------------------------------

pub fn load_schedules() -> Vec<ScheduleEntry> {
    let path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("octotrack")
        .join("schedules.json");

    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    match value.as_array() {
        Some(arr) => arr.iter().filter_map(ScheduleEntry::from_json).collect(),
        None => vec![],
    }
}

// ---------------------------------------------------------------------------
// Background scheduler thread
// ---------------------------------------------------------------------------

/// Spawns a thread that fires `ScheduleMsg` events at scheduled times.
/// The thread sleeps to the top of each minute, then checks all entries.
pub fn run_scheduler(entries: Vec<ScheduleEntry>, sender: mpsc::Sender<ScheduleMsg>) {
    thread::spawn(move || {
        if entries.is_empty() {
            return;
        }
        // Keep a small debounce queue so the same schedule can't fire twice in
        // the same minute if there's a tiny timing slip (e.g. sleep overshoots).
        let mut last_fired: VecDeque<(u32, u32, ScheduleAction)> = VecDeque::new();

        loop {
            // Sleep until the start of the next minute.
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let secs_to_next = 60 - (now_secs % 60);
            thread::sleep(Duration::from_secs(secs_to_next));

            let t = local_now();
            let (min, hour, day, month, wday) = t;

            // Prune the debounce queue (keep only entries from this exact minute).
            last_fired.retain(|&(m, h, _)| m == min && h == hour);

            for entry in &entries {
                if entry.cron.matches(min, hour, day, month, wday) {
                    // Skip if we already fired this action this minute.
                    if last_fired.iter().any(|&(m, h, a)| m == min && h == hour && a == entry.action) {
                        continue;
                    }
                    last_fired.push_back((min, hour, entry.action));

                    let tx = sender.clone();
                    let action = entry.action;
                    let dur = entry.duration_secs;

                    let _ = tx.send(ScheduleMsg::Start(action));

                    // Spawn a short-lived thread to send the Stop after the duration.
                    thread::spawn(move || {
                        thread::sleep(Duration::from_secs(dur));
                        let _ = tx.send(ScheduleMsg::Stop(action));
                    });
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Local time  (uses libc so we avoid adding a chrono dependency)
// ---------------------------------------------------------------------------

/// Returns `(minute, hour, day-of-month, month 1-12, weekday 0=Sunday)`.
fn local_now() -> (u32, u32, u32, u32, u32) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as libc::time_t;
    let mut t: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&ts, &mut t) };
    (
        t.tm_min as u32,
        t.tm_hour as u32,
        t.tm_mday as u32,
        (t.tm_mon + 1) as u32, // tm_mon is 0-based
        t.tm_wday as u32,      // 0 = Sunday
    )
}

// ---------------------------------------------------------------------------
// 5-field cron expression parser and matcher
// ---------------------------------------------------------------------------
// Format: minute  hour  day-of-month  month  day-of-week
// Each field supports: *  N  N-M  */N  N-M/N  and comma-separated lists.

#[derive(Debug)]
pub struct CronExpr {
    minutes:  Vec<u32>, // 0-59
    hours:    Vec<u32>, // 0-23
    days:     Vec<u32>, // 1-31
    months:   Vec<u32>, // 1-12
    weekdays: Vec<u32>, // 0-6  (0 = Sunday)
}

impl CronExpr {
    pub fn parse(s: &str) -> Option<Self> {
        let f: Vec<&str> = s.split_whitespace().collect();
        if f.len() != 5 {
            return None;
        }
        Some(CronExpr {
            minutes:  expand_field(f[0], 0, 59)?,
            hours:    expand_field(f[1], 0, 23)?,
            days:     expand_field(f[2], 1, 31)?,
            months:   expand_field(f[3], 1, 12)?,
            weekdays: expand_field(f[4], 0, 6)?,
        })
    }

    pub fn matches(&self, min: u32, hour: u32, day: u32, month: u32, wday: u32) -> bool {
        self.minutes.contains(&min)
            && self.hours.contains(&hour)
            && self.days.contains(&day)
            && self.months.contains(&month)
            && self.weekdays.contains(&wday)
    }
}

fn expand_field(s: &str, lo: u32, hi: u32) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        expand_part(part.trim(), lo, hi, &mut out)?;
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

fn expand_part(s: &str, lo: u32, hi: u32, out: &mut Vec<u32>) -> Option<()> {
    if s == "*" {
        out.extend(lo..=hi);
        return Some(());
    }
    // */step
    if let Some(step_s) = s.strip_prefix("*/") {
        let step: u32 = step_s.parse().ok()?;
        if step == 0 { return None; }
        out.extend((lo..=hi).step_by(step as usize));
        return Some(());
    }
    // range/step  or  range  or  single
    if let Some((range_s, step_s)) = s.split_once('/') {
        let step: u32 = step_s.parse().ok()?;
        if step == 0 { return None; }
        let (a, b) = parse_range(range_s)?;
        out.extend((a..=b).step_by(step as usize));
        return Some(());
    }
    if s.contains('-') {
        let (a, b) = parse_range(s)?;
        out.extend(a..=b);
        return Some(());
    }
    let v: u32 = s.parse().ok()?;
    out.push(v);
    Some(())
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let (a_s, b_s) = s.split_once('-')?;
    let a: u32 = a_s.trim().parse().ok()?;
    let b: u32 = b_s.trim().parse().ok()?;
    Some((a, b))
}

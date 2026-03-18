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

#[derive(Debug)]
pub enum ScheduleMsg {
    Start {
        action: ScheduleAction,
        /// Filename (or partial name) to select before starting playback.
        start_track: Option<String>,
    },
    Stop(ScheduleAction),
}

// ---------------------------------------------------------------------------
// Schedule entry
// ---------------------------------------------------------------------------

pub struct ScheduleEntry {
    pub cron: CronExpr,
    pub action: ScheduleAction,
    pub duration_secs: u64,
    /// Filename (or partial name) to select when this schedule fires.
    /// Only meaningful for `action: "play"`.
    pub start_track: Option<String>,
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
        let start_track = v["start_track"].as_str().map(|s| s.to_string());
        Some(ScheduleEntry {
            cron,
            action,
            duration_secs,
            start_track,
        })
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
        // Debounce per entry index: prevents a single entry from double-firing if
        // the sleep overshoots a minute boundary. Keyed by (entry_index, min, hour).
        let mut last_fired: VecDeque<(usize, u32, u32)> = VecDeque::new();

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

            // Prune entries that are no longer in this minute.
            last_fired.retain(|&(_, m, h)| m == min && h == hour);

            for (idx, entry) in entries.iter().enumerate() {
                if entry.cron.matches(min, hour, day, month, wday) {
                    // Skip if this specific entry already fired this minute.
                    if last_fired
                        .iter()
                        .any(|&(i, m, h)| i == idx && m == min && h == hour)
                    {
                        continue;
                    }
                    last_fired.push_back((idx, min, hour));

                    let tx = sender.clone();
                    let action = entry.action;
                    let dur = entry.duration_secs;
                    let start_track = entry.start_track.clone();

                    let _ = tx.send(ScheduleMsg::Start {
                        action,
                        start_track,
                    });

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
    minutes: Vec<u32>,  // 0-59
    hours: Vec<u32>,    // 0-23
    days: Vec<u32>,     // 1-31
    months: Vec<u32>,   // 1-12
    weekdays: Vec<u32>, // 0-6  (0 = Sunday)
}

impl CronExpr {
    pub fn parse(s: &str) -> Option<Self> {
        let f: Vec<&str> = s.split_whitespace().collect();
        if f.len() != 5 {
            return None;
        }
        Some(CronExpr {
            minutes: expand_field(f[0], 0, 59)?,
            hours: expand_field(f[1], 0, 23)?,
            days: expand_field(f[2], 1, 31)?,
            months: expand_field(f[3], 1, 12)?,
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
        if step == 0 {
            return None;
        }
        out.extend((lo..=hi).step_by(step as usize));
        return Some(());
    }
    // range/step  or  range  or  single
    if let Some((range_s, step_s)) = s.split_once('/') {
        let step: u32 = step_s.parse().ok()?;
        if step == 0 {
            return None;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // CronExpr::parse
    // -----------------------------------------------------------------------

    #[test]
    fn parse_every_minute() {
        let c = CronExpr::parse("* * * * *").unwrap();
        assert_eq!(c.minutes.len(), 60);
        assert_eq!(c.hours.len(), 24);
        assert_eq!(c.days.len(), 31);
        assert_eq!(c.months.len(), 12);
        assert_eq!(c.weekdays.len(), 7);
    }

    #[test]
    fn parse_specific_values() {
        let c = CronExpr::parse("30 14 25 12 5").unwrap();
        assert_eq!(c.minutes, vec![30]);
        assert_eq!(c.hours, vec![14]);
        assert_eq!(c.days, vec![25]);
        assert_eq!(c.months, vec![12]);
        assert_eq!(c.weekdays, vec![5]);
    }

    #[test]
    fn parse_ranges() {
        let c = CronExpr::parse("0-5 9-17 1-15 1-6 1-5").unwrap();
        assert_eq!(c.minutes, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(c.hours, vec![9, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(c.weekdays, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn parse_step() {
        let c = CronExpr::parse("*/15 */2 * * *").unwrap();
        assert_eq!(c.minutes, vec![0, 15, 30, 45]);
        assert_eq!(c.hours, vec![0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22]);
    }

    #[test]
    fn parse_range_with_step() {
        let c = CronExpr::parse("0-30/10 * * * *").unwrap();
        assert_eq!(c.minutes, vec![0, 10, 20, 30]);
    }

    #[test]
    fn parse_comma_list() {
        let c = CronExpr::parse("0,15,30,45 * * * *").unwrap();
        assert_eq!(c.minutes, vec![0, 15, 30, 45]);
    }

    #[test]
    fn parse_wrong_field_count() {
        assert!(CronExpr::parse("* * *").is_none());
        assert!(CronExpr::parse("* * * * * *").is_none());
        assert!(CronExpr::parse("").is_none());
    }

    #[test]
    fn parse_invalid_values() {
        assert!(CronExpr::parse("abc * * * *").is_none());
        assert!(CronExpr::parse("*/0 * * * *").is_none());
    }

    // -----------------------------------------------------------------------
    // CronExpr::matches
    // -----------------------------------------------------------------------

    #[test]
    fn matches_exact() {
        let c = CronExpr::parse("0 1 * * *").unwrap();
        assert!(c.matches(0, 1, 15, 6, 3));
        assert!(!c.matches(1, 1, 15, 6, 3));
        assert!(!c.matches(0, 2, 15, 6, 3));
    }

    #[test]
    fn matches_weekday_range() {
        let c = CronExpr::parse("0 22 * * 1-5").unwrap();
        // Monday=1 through Friday=5
        assert!(c.matches(0, 22, 1, 1, 1));
        assert!(c.matches(0, 22, 1, 1, 5));
        // Sunday=0, Saturday=6
        assert!(!c.matches(0, 22, 1, 1, 0));
        assert!(!c.matches(0, 22, 1, 1, 6));
    }

    #[test]
    fn matches_every_15_min() {
        let c = CronExpr::parse("*/15 * * * *").unwrap();
        assert!(c.matches(0, 0, 1, 1, 0));
        assert!(c.matches(15, 12, 1, 1, 0));
        assert!(c.matches(30, 12, 1, 1, 0));
        assert!(c.matches(45, 12, 1, 1, 0));
        assert!(!c.matches(10, 12, 1, 1, 0));
    }

    // -----------------------------------------------------------------------
    // ScheduleEntry::from_json
    // -----------------------------------------------------------------------

    #[test]
    fn entry_from_json_rec() {
        let v: serde_json::Value = serde_json::json!({
            "cron": "0 1 * * *",
            "action": "rec",
            "duration_minutes": 60
        });
        let e = ScheduleEntry::from_json(&v).unwrap();
        assert_eq!(e.action, ScheduleAction::Rec);
        assert_eq!(e.duration_secs, 3600);
        assert!(e.start_track.is_none());
    }

    #[test]
    fn entry_from_json_play_with_track() {
        let v: serde_json::Value = serde_json::json!({
            "cron": "0 22 * * 1-5",
            "action": "play",
            "duration_seconds": 300,
            "start_track": "evening_set"
        });
        let e = ScheduleEntry::from_json(&v).unwrap();
        assert_eq!(e.action, ScheduleAction::Play);
        assert_eq!(e.duration_secs, 300);
        assert_eq!(e.start_track.as_deref(), Some("evening_set"));
    }

    #[test]
    fn entry_from_json_missing_duration() {
        let v: serde_json::Value = serde_json::json!({
            "cron": "0 1 * * *",
            "action": "rec"
        });
        assert!(ScheduleEntry::from_json(&v).is_none());
    }

    #[test]
    fn entry_from_json_invalid_action() {
        let v: serde_json::Value = serde_json::json!({
            "cron": "0 1 * * *",
            "action": "nope",
            "duration_minutes": 60
        });
        assert!(ScheduleEntry::from_json(&v).is_none());
    }

    #[test]
    fn entry_from_json_invalid_cron() {
        let v: serde_json::Value = serde_json::json!({
            "cron": "not a cron",
            "action": "rec",
            "duration_minutes": 60
        });
        assert!(ScheduleEntry::from_json(&v).is_none());
    }

    // -----------------------------------------------------------------------
    // expand_field / expand_part edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn expand_field_deduplicates() {
        // "0,0,0" should produce just [0]
        let result = expand_field("0,0,0", 0, 59).unwrap();
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn expand_field_sorts() {
        let result = expand_field("30,10,20", 0, 59).unwrap();
        assert_eq!(result, vec![10, 20, 30]);
    }

    #[test]
    fn range_step_zero_returns_none() {
        assert!(expand_field("1-10/0", 0, 59).is_none());
    }
}

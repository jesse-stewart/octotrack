# Scheduled Tasks

Octotrack can start and stop playback or recording automatically on a cron-style schedule. Schedules are stored in `~/.config/octotrack/schedules.json` — create this file manually (it is not generated automatically).

## Schedule file format

```json
[
  {
    "cron": "0 1 * * *",
    "action": "rec",
    "duration_minutes": 60
  },
  {
    "cron": "0 22 * * 1-5",
    "action": "play",
    "duration_minutes": 120,
    "start_track": "evening_set"
  }
]
```

Any number of entries can be added to the array. Each entry fires independently — two entries with the same action at the same time will both fire.

Each entry supports the following fields:

| Field | Required | Description |
|---|---|---|
| `cron` | yes | 5-field cron expression: `minute hour day-of-month month day-of-week` |
| `action` | yes | `"rec"` to start/stop recording, `"play"` to start/stop playback |
| `duration_minutes` | one of these | How long to run before stopping automatically |
| `duration_seconds` | one of these | Same as above, for sub-minute precision |
| `start_track` | no | Filename (or partial name) to select before starting playback. Case-insensitive substring match — e.g. `"evening_set"` selects the first track whose filename contains that string. Ignored for `"rec"` actions. |

## Cron expression format

Standard 5-field cron — the same format used by Unix cron:

```
┌─ minute       (0-59)
│ ┌─ hour        (0-23)
│ │ ┌─ day        (1-31)
│ │ │ ┌─ month     (1-12)
│ │ │ │ ┌─ weekday  (0-6, 0=Sunday)
│ │ │ │ │
* * * * *
```

Each field supports:

| Syntax | Meaning | Example |
|---|---|---|
| `*` | Every value | `*` in hours = every hour |
| `N` | Specific value | `5` in minutes = at :05 |
| `N-M` | Range | `1-5` in weekdays = Mon–Fri |
| `*/N` | Every Nth | `*/2` in hours = every 2 hours |
| `N-M/N` | Range with step | `0-20/2` in hours = 0,2,4,…,20 |
| `N,M,…` | List | `0,15,30,45` in minutes = every 15 min |

**Examples:**

| Expression | Meaning |
|---|---|
| `0 1 * * *` | Every day at 01:00 |
| `0 22 * * 1-5` | Weekdays at 22:00 |
| `23 0-20/2 * * *` | Minute :23 past every 2nd hour, midnight through 20:00 |
| `0 9 * * 1` | Every Monday at 09:00 |
| `0 8,20 * * *` | Every day at 08:00 and 20:00 |
| `0 0 25 12 *` | December 25th at midnight (repeats yearly) |

**Specific date/time:** Standard cron doesn't have a year field, so you can't schedule a truly one-off event. The closest you can get is being precise about day and month (e.g. `0 14 25 12 *` fires at 14:00 every December 25th). To run something once at a specific time, set the schedule, let it fire, then remove the entry from `schedules.json`.

## How schedules interact with `auto_mode`

The `auto_mode` setting in `config.toml` controls what happens at **startup** only. Scheduled tasks are independent — `action: "rec"` always starts recording regardless of what `auto_mode` is set to.

Multiple schedules can be active at once. If two schedules overlap, both will fire independently.

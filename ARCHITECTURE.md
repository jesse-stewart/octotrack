# Octotrack Architecture

## 1. High-Level Component Overview

```mermaid
graph TD
    main["main.rs\nEvent Loop"] --> app["App\nState Machine"]
    main --> tui["Tui\nTerminal Lifecycle"]
    main --> events["EventHandler\nBackground Thread"]
    main --> sched["Scheduler\nBackground Thread"]

    app --> audio["AudioPlayer\nAudio I/O Orchestrator"]
    app --> cfg["Config\n~/.config/octotrack/\nconfig.json"]

    audio --> mplayer["mplayer\n(playback)"]
    audio --> ffmpeg["ffmpeg\n(merge / analysis)"]
    audio --> arecord["arecord\n(capture)"]
    audio --> aplay["aplay\n(monitoring)"]

    tui --> ui["ui.rs\nRatatui Layout & Widgets"]
    events -->|"mpsc Event"| main
    sched -->|"mpsc ScheduleMsg"| main
```

---

## 2. Main Event Loop Flow

```mermaid
flowchart TD
    start([Start]) --> init["Init terminal\nLoad config\nDiscover tracks"]
    init --> loop_top["Loop iteration"]
    loop_top --> update["1. Update playback info\n(position, levels, rec elapsed)"]
    update --> render["2. Render TUI\n(ratatui → terminal)"]
    render --> poll["3. Poll for events"]
    poll --> tick{"Event?"}
    tick -->|"Tick"| sched_check["Check ScheduleMsg\n(cron actions)"]
    sched_check --> loop_top
    tick -->|"KeyEvent"| handler["handler.rs\nMap key → App method"]
    handler --> loop_top
    tick -->|"Quit confirmed"| exit([Exit])
```

---

## 3. Audio Pipeline

```mermaid
graph LR
    subgraph Playback
        p1["App.play()"] --> fifo["/tmp/octotrack_mplayer.fifo"]
        p1 --> ffmpeg_merge["ffmpeg\n(multi-file merge)"]
        ffmpeg_merge --> mplayer_proc["mplayer process\n(ALSA output)"]
        fifo --> mplayer_proc
        mplayer_proc --> ffmpeg_analyze["ffmpeg PCM pipe\n(level analysis)"]
        ffmpeg_analyze --> levels_play["Arc<Mutex<Vec<f32>>>\nchannel_levels"]
    end

    subgraph Capture
        p2["App.start_recording()\nor start_monitoring()"] --> arecord_proc["arecord process\n(ALSA input)"]
        arecord_proc --> capture_thread["Capture Thread\n(reads stdout)"]
        capture_thread --> wav_file["WAV / RF64 file"]
        capture_thread --> aplay_proc["aplay process\n(monitoring output)"]
        capture_thread --> levels_rec["Arc<Mutex<Vec<f32>>>\nchannel_levels"]
    end
```

---

## 4. App State Machine

```mermaid
stateDiagram-v2
    [*] --> Idle

    Idle --> Playing : Space / AutoMode=Play
    Playing --> Idle : S (stop)
    Playing --> Playing : ← → (prev/next track)

    Idle --> Recording : R
    Recording --> Idle : R (stop_recording)

    Recording --> RecordingWithMonitor : M
    RecordingWithMonitor --> Recording : M

    Idle --> Monitoring : M (standalone)
    Monitoring --> Idle : M

    note right of Recording
        Capture thread running
        WAV file being written
        File split / drop-mode rolling
    end note

    note right of Playing
        mplayer slave process
        FIFO for runtime control
        LoopMode: NoLoop / Single / All
    end note
```

---

## 5. Threading Model

```mermaid
graph TD
    main_thread["Main Thread\n(event loop + UI)"]

    subgraph Background Threads
        evt_thread["EventHandler Thread\ncrossterm poll"]
        sched_thread["Scheduler Thread\ncron matching, sleeps to minute boundary"]
        capture_thread["Capture Thread\narecord stdout reader, WAV writer"]
        analyze_play["Playback Analyzer Thread\nffmpeg PCM → RMS levels"]
        stop_timer["Stop-timer Thread(s)\nsleep N sec → send Stop"]
    end

    evt_thread -->|"mpsc<Event>"| main_thread
    sched_thread -->|"mpsc<ScheduleMsg>"| main_thread
    sched_thread --> stop_timer
    stop_timer -->|"mpsc<ScheduleMsg>"| main_thread

    capture_thread -->|"Arc<Mutex<Vec<f32>>>"| main_thread
    analyze_play -->|"Arc<Mutex<Vec<f32>>>"| main_thread
```

---

## 6. Key Data Structures & Relationships

```mermaid
classDiagram
    class App {
        +track_list: Vec~String~
        +current_track_index: usize
        +is_playing: bool
        +is_recording: bool
        +is_monitoring: bool
        +loop_mode: LoopMode
        +auto_mode: AutoMode
        +eq_bands: Vec~i32~
        +volume: u8
        +player: AudioPlayer
        +play()
        +stop()
        +start_recording()
        +stop_recording()
        +start_monitoring()
    }

    class AudioPlayer {
        +channel_levels: Arc~Mutex~Vec~f32~~~
        +play_file()
        +stop()
        +start_capture()
        +stop_capture()
        +enable_monitor_output()
    }

    class ScheduleEntry {
        +cron: CronExpr
        +action: ScheduleAction
        +duration_min: Option~u64~
        +track: Option~String~
    }

    class CronExpr {
        +minute: Vec~u8~
        +hour: Vec~u8~
        +dom: Vec~u8~
        +month: Vec~u8~
        +dow: Vec~u8~
        +matches(now)
    }

    class RecordingConfig {
        +channel_count: u16
        +sample_rate: u32
        +bit_depth: u16
        +max_data_bytes: Option~u64~
        +max_mode: RecMaxMode
        +split_bytes: Option~u64~
        +min_free_bytes: u64
    }

    App --> AudioPlayer
    App --> ScheduleEntry
    AudioPlayer --> RecordingConfig
    ScheduleEntry --> CronExpr
```

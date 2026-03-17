use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

#[cfg(unix)]
extern crate libc;

fn alsa_format(bit_depth: u16) -> &'static str {
    match bit_depth {
        16 => "S16_LE",
        24 => "S24_3LE",
        _ => "S32_LE",
    }
}

fn bytes_per_sample(bit_depth: u16) -> usize {
    match bit_depth {
        16 => 2,
        24 => 3,
        _ => 4,
    }
}

fn log_path() -> PathBuf {
    std::env::temp_dir().join("octotrack.log")
}

fn log(msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

fn log_stdio() -> Stdio {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

pub struct RecordingConfig {
    pub max_data_bytes: Option<u64>,
    pub drop_mode: bool,
    pub min_free_bytes: u64,
}

pub struct AudioPlayer {
    process: Option<Child>,
    ffmpeg_process: Option<Child>,
    control_fifo: Option<File>,
    fifo_path: PathBuf,
    analysis_process: Option<Child>,
    start_time: Option<Instant>,
    channel_levels: Arc<Mutex<Vec<f32>>>,
    current_volume: u8,
    // Unified capture: one arecord serves both recording and monitoring.
    capture_arecord: Option<Child>,
    capture_aplay: Option<Child>,
    capture_thread: Option<thread::JoinHandle<()>>,
    // Shared with the capture thread. Set Some(stdin) to route audio to aplay.
    capture_monitor_sink: Arc<Mutex<Option<ChildStdin>>>,
    capture_is_recording: bool,
    pub capture_recording_bytes: Arc<Mutex<u64>>,
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPlayer {
    pub fn new() -> Self {
        let fifo_path = std::env::temp_dir().join("octotrack_mplayer.fifo");
        AudioPlayer {
            process: None,
            ffmpeg_process: None,
            control_fifo: None,
            fifo_path,
            analysis_process: None,
            start_time: None,
            channel_levels: Arc::new(Mutex::new(Vec::new())),
            current_volume: 100,
            capture_arecord: None,
            capture_aplay: None,
            capture_thread: None,
            capture_monitor_sink: Arc::new(Mutex::new(None)),
            capture_is_recording: false,
            capture_recording_bytes: Arc::new(Mutex::new(0)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn play(
        &mut self,
        track_path: &PathBuf,
        channel_count: u32,
        output_channel_count: u32,
        volume: u8,
        max_volume: u8,
        eq_bands: &[i8; 10],
        eq_enabled: bool,
        playback_device: &str,
    ) -> io::Result<()> {
        log(&format!(
            "play: path={} channels={} output_channels={} device={}",
            track_path.display(),
            channel_count,
            output_channel_count,
            playback_device
        ));
        // Ensure any existing process is stopped before starting a new one
        if let Some(mut process) = self.process.take() {
            process.kill()?;
            process.wait()?;
        }
        if let Some(mut ffmpeg_process) = self.ffmpeg_process.take() {
            ffmpeg_process.kill()?;
            ffmpeg_process.wait()?;
        }
        if let Some(mut analysis_process) = self.analysis_process.take() {
            analysis_process.kill()?;
            analysis_process.wait()?;
        }
        // Close any open FIFO
        self.control_fifo = None;

        // Setup control FIFO for slave commands
        self.setup_control_fifo()?;

        // Store current volume
        self.current_volume = volume;

        // Initialize channel levels
        *self.channel_levels.lock().unwrap() = vec![-60.0; channel_count as usize];

        // Start audio analysis in real-time
        self.start_audio_analysis(track_path, channel_count)?;

        // Set start time
        self.start_time = Some(Instant::now());

        // Check if track_path is a directory with multiple audio files
        if track_path.is_dir() {
            self.play_multi_file(
                track_path,
                output_channel_count,
                volume,
                max_volume,
                eq_bands,
                eq_enabled,
                playback_device,
            )
        } else {
            self.play_single_file(
                track_path,
                output_channel_count,
                volume,
                max_volume,
                eq_bands,
                eq_enabled,
                playback_device,
            )
        }
    }

    fn setup_control_fifo(&mut self) -> io::Result<()> {
        // Remove existing FIFO if it exists
        let _ = std::fs::remove_file(&self.fifo_path);

        // Create new FIFO using mkfifo command
        let status = Command::new("mkfifo").arg(&self.fifo_path).status()?;

        if !status.success() {
            return Err(io::Error::other("Failed to create FIFO"));
        }

        Ok(())
    }

    fn start_audio_analysis(&mut self, track_path: &PathBuf, channel_count: u32) -> io::Result<()> {
        let mut cmd = Command::new("ffmpeg");

        // Use -re for real-time processing
        cmd.arg("-re");

        // Handle both single files and directories
        if track_path.is_dir() {
            // Get all audio files in the directory
            let mut audio_files: Vec<PathBuf> = std::fs::read_dir(track_path)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("mp3")
                            || ext.eq_ignore_ascii_case("wav")
                            || ext.eq_ignore_ascii_case("flac")
                    })
                })
                .collect();

            audio_files.sort();

            if audio_files.is_empty() {
                return Ok(());
            }

            // Add all input files
            for file in &audio_files {
                cmd.arg("-i").arg(file);
            }

            // Build filter complex to merge audio
            let input_count = audio_files.len();
            let mut filter_complex = String::new();
            for i in 0..input_count {
                filter_complex.push_str(&format!("[{}:a]", i));
            }
            filter_complex.push_str(&format!("amerge=inputs={}", input_count));

            cmd.arg("-filter_complex").arg(&filter_complex);
        } else {
            // Single file
            cmd.arg("-i").arg(track_path);
        }

        // Output as raw PCM audio: s16le format, native sample rate
        cmd.arg("-f")
            .arg("s16le")
            .arg("-ac")
            .arg(channel_count.to_string())
            .arg("-")
            .arg("-loglevel")
            .arg("error");

        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Failed to capture stdout"))?;

        let levels = Arc::clone(&self.channel_levels);
        let channel_count = channel_count as usize;

        // Spawn a thread to analyze the PCM audio data
        thread::spawn(move || {
            let mut reader = stdout;
            let mut buffer = vec![0u8; 4096 * channel_count * 2]; // Buffer for samples (2 bytes per sample)
            let mut current_levels = vec![-60.0; channel_count];

            // RMS calculation buffers (100ms windows ~= 4800 samples at 48kHz)
            let window_samples = 4800;
            let mut channel_buffers: Vec<Vec<f32>> = vec![vec![]; channel_count];

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        // Process samples (s16le = 2 bytes per sample)
                        let num_samples = n / 2 / channel_count;

                        for i in 0..num_samples {
                            #[allow(clippy::needless_range_loop)]
                            for ch in 0..channel_count {
                                let offset = (i * channel_count + ch) * 2;
                                if offset + 1 < n {
                                    // Read s16le sample
                                    let sample =
                                        i16::from_le_bytes([buffer[offset], buffer[offset + 1]]);
                                    // Convert to float (-1.0 to 1.0)
                                    let sample_f = sample as f32 / 32768.0;
                                    channel_buffers[ch].push(sample_f);
                                }
                            }
                        }

                        // Calculate RMS for each channel when we have enough samples
                        let mut updated = false;
                        #[allow(clippy::needless_range_loop)]
                        for ch in 0..channel_count {
                            if channel_buffers[ch].len() >= window_samples {
                                // Calculate RMS
                                let sum_squares: f32 =
                                    channel_buffers[ch].iter().map(|&s| s * s).sum();
                                let rms = (sum_squares / channel_buffers[ch].len() as f32).sqrt();

                                // Convert to dB
                                let db = if rms > 0.0 { 20.0 * rms.log10() } else { -60.0 };

                                current_levels[ch] = db.clamp(-60.0, 0.0);
                                updated = true;

                                // Clear buffer for next window
                                channel_buffers[ch].clear();
                            }
                        }

                        // Update shared levels
                        if updated {
                            if let Ok(mut levels_guard) = levels.lock() {
                                *levels_guard = current_levels.clone();
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        self.analysis_process = Some(child);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn play_single_file(
        &mut self,
        file_path: &PathBuf,
        output_channel_count: u32,
        volume: u8,
        max_volume: u8,
        eq_bands: &[i8; 10],
        eq_enabled: bool,
        playback_device: &str,
    ) -> io::Result<()> {
        // Use mplayer's built-in volume control with slave mode for dynamic control
        let mut cmd = Command::new("mplayer");
        cmd.arg("-slave")
            .arg("-quiet")
            .arg("-input")
            .arg(format!("file={}", self.fifo_path.display()))
            .arg("-channels")
            .arg(output_channel_count.to_string())
            .arg("-softvol")
            .arg("-softvol-max")
            .arg(max_volume.to_string())
            .arg("-volume")
            .arg(volume.to_string());

        // Add EQ filter if enabled
        if eq_enabled {
            cmd.arg("-af").arg(Self::eq_filter_string(eq_bands));
        }

        let process = cmd
            .arg("-ao")
            .arg(format!(
                "alsa:device={}",
                playback_device.replace("hw:", "plughw=").replace(',', ".")
            ))
            .arg(file_path)
            .stdin(Stdio::null())
            .stdout(log_stdio())
            .stderr(log_stdio())
            .spawn()?;

        self.process = Some(process);
        log("play_single_file: mplayer spawned");

        // Give mplayer a moment to open the FIFO
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Open FIFO for writing
        self.control_fifo = Some(OpenOptions::new().write(true).open(&self.fifo_path)?);

        log("play_single_file: fifo opened, playback started");
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn play_multi_file(
        &mut self,
        folder_path: &PathBuf,
        output_channel_count: u32,
        volume: u8,
        max_volume: u8,
        eq_bands: &[i8; 10],
        eq_enabled: bool,
        playback_device: &str,
    ) -> io::Result<()> {
        // Get all audio files in the directory, sorted
        let mut audio_files: Vec<PathBuf> = std::fs::read_dir(folder_path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("mp3")
                        || ext.eq_ignore_ascii_case("wav")
                        || ext.eq_ignore_ascii_case("flac")
                })
            })
            .collect();

        audio_files.sort();

        if audio_files.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "No audio files found in directory",
            ));
        }

        // If only one file, play it directly
        if audio_files.len() == 1 {
            return self.play_single_file(
                &audio_files[0],
                output_channel_count,
                volume,
                max_volume,
                eq_bands,
                eq_enabled,
                playback_device,
            );
        }

        // Build ffmpeg command to merge multiple audio files
        let mut ffmpeg_cmd = Command::new("ffmpeg");

        // Add all input files
        for file in &audio_files {
            ffmpeg_cmd.arg("-i").arg(file);
        }

        // Build the filter complex to merge all audio streams
        let input_count = audio_files.len();
        let mut filter_complex = String::new();
        for i in 0..input_count {
            filter_complex.push_str(&format!("[{}:a]", i));
        }
        filter_complex.push_str(&format!("amerge=inputs={}[aout]", input_count));

        ffmpeg_cmd
            .arg("-filter_complex")
            .arg(&filter_complex)
            .arg("-map")
            .arg("[aout]")
            .arg("-f")
            .arg("wav")
            .arg("-");

        // Pipe ffmpeg output to mplayer with volume control
        let mut ffmpeg_output = ffmpeg_cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let mplayer_stdin = ffmpeg_output
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Failed to capture ffmpeg stdout"))?;

        let mut mplayer_cmd = Command::new("mplayer");
        mplayer_cmd
            .arg("-slave")
            .arg("-quiet")
            .arg("-input")
            .arg(format!("file={}", self.fifo_path.display()))
            .arg("-channels")
            .arg(output_channel_count.to_string())
            .arg("-softvol")
            .arg("-softvol-max")
            .arg(max_volume.to_string())
            .arg("-volume")
            .arg(volume.to_string());

        if eq_enabled {
            mplayer_cmd.arg("-af").arg(Self::eq_filter_string(eq_bands));
        }

        let mplayer_process = mplayer_cmd
            .arg("-ao")
            .arg(format!(
                "alsa:device={}",
                playback_device.replace("hw:", "plughw=").replace(',', ".")
            ))
            .arg("-")
            .stdin(mplayer_stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        self.ffmpeg_process = Some(ffmpeg_output);
        self.process = Some(mplayer_process);

        // Give mplayer a moment to open the FIFO
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Open FIFO for writing
        self.control_fifo = Some(OpenOptions::new().write(true).open(&self.fifo_path)?);

        Ok(())
    }

    fn eq_filter_string(bands: &[i8; 10]) -> String {
        let values: Vec<String> = bands.iter().map(|b| b.to_string()).collect();
        format!("equalizer={}", values.join(":"))
    }

    /// Update EQ band values smoothly during playback
    pub fn update_eq_bands(&mut self, bands: &[i8; 10]) -> io::Result<()> {
        if let Some(fifo) = &mut self.control_fifo {
            let values: Vec<String> = bands.iter().map(|b| b.to_string()).collect();
            writeln!(
                fifo,
                "pausing_keep_force af_cmdline equalizer {}",
                values.join(":")
            )?;
            fifo.flush()?;
        }
        Ok(())
    }

    /// Toggle EQ on/off (for bypass - requires delete+add)
    pub fn set_eq_enabled(&mut self, bands: &[i8; 10], enabled: bool) -> io::Result<()> {
        if let Some(fifo) = &mut self.control_fifo {
            writeln!(fifo, "pausing_keep_force af_del equalizer")?;
            fifo.flush()?;

            if enabled {
                writeln!(
                    fifo,
                    "pausing_keep_force af_add {}",
                    Self::eq_filter_string(bands)
                )?;
                fifo.flush()?;
            }
        }
        Ok(())
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if let Some(mut process) = self.process.take() {
            process.kill()?;
            process.wait()?;
        }
        if let Some(mut ffmpeg_process) = self.ffmpeg_process.take() {
            ffmpeg_process.kill()?;
            ffmpeg_process.wait()?;
        }
        if let Some(mut analysis_process) = self.analysis_process.take() {
            analysis_process.kill()?;
            analysis_process.wait()?;
        }
        // Close FIFO and clean up
        self.control_fifo = None;
        self.start_time = None;
        let _ = std::fs::remove_file(&self.fifo_path);
        Ok(())
    }

    pub fn is_running(&mut self) -> bool {
        if let Some(process) = &mut self.process {
            match process.try_wait() {
                Ok(Some(status)) => {
                    log(&format!("is_running: process exited with {}", status));
                    // Clean up ffmpeg process if mplayer has exited
                    if let Some(mut ffmpeg_process) = self.ffmpeg_process.take() {
                        let _ = ffmpeg_process.kill();
                        let _ = ffmpeg_process.wait();
                    }
                    false
                }
                Ok(None) => true, // Process is still running
                Err(_) => false,  // Error in checking, assume not running
            }
        } else {
            false
        }
    }

    pub fn set_volume(&mut self, volume: u8) -> io::Result<()> {
        self.current_volume = volume;
        if let Some(fifo) = &mut self.control_fifo {
            // Send mplayer slave command to set absolute volume
            // Use pausing_keep_force to ensure command works during playback
            writeln!(fifo, "pausing_keep_force volume {} 1", volume)?;
            fifo.flush()?;
        }
        Ok(())
    }

    pub fn get_time_pos(&self) -> io::Result<f32> {
        if let Some(start_time) = self.start_time {
            let elapsed = start_time.elapsed().as_secs_f32();
            Ok(elapsed)
        } else {
            Ok(0.0)
        }
    }

    pub fn start_recording(
        &mut self,
        output_path: &Path,
        input_device: &str,
        channel_count: u32,
        sample_rate: u32,
        bit_depth: u16,
        rec_cfg: RecordingConfig,
    ) -> io::Result<()> {
        log(&format!(
            "start_recording: device={} channels={} rate={} bits={} max={:?} drop={} min_free={} path={:?}",
            input_device, channel_count, sample_rate, bit_depth, rec_cfg.max_data_bytes, rec_cfg.drop_mode, rec_cfg.min_free_bytes, output_path
        ));
        self.stop_capture()?;

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        *self.channel_levels.lock().unwrap() = vec![-60.0; channel_count as usize];
        self.start_capture_internal(input_device, channel_count, sample_rate, bit_depth, Some(output_path.to_path_buf()), None, rec_cfg)?;
        self.capture_is_recording = true;
        Ok(())
    }

    pub fn stop_recording(&mut self) -> io::Result<()> {
        log("stop_recording called");
        self.capture_is_recording = false;
        self.stop_capture()
    }

    pub fn is_recording(&mut self) -> bool {
        if !self.capture_is_recording {
            return false;
        }
        match &mut self.capture_arecord {
            None => {
                self.capture_is_recording = false;
                false
            }
            Some(p) => match p.try_wait() {
                Ok(None) => true,
                _ => {
                    self.capture_arecord = None;
                    self.capture_is_recording = false;
                    false
                }
            },
        }
    }

    pub fn start_monitoring(
        &mut self,
        input_device: &str,
        output_device: &str,
        channel_count: u32,
        sample_rate: u32,
        bit_depth: u16,
    ) -> io::Result<()> {
        log(&format!(
            "start_monitoring: in={} out={} channels={} rate={} bits={}",
            input_device, output_device, channel_count, sample_rate, bit_depth
        ));

        if self.capture_is_recording && self.capture_arecord.is_some() {
            // Recording is active — add monitoring output without interrupting capture.
            return self.enable_monitor_output(output_device, channel_count, sample_rate, bit_depth);
        }

        // No active capture — start a monitoring-only session.
        self.stop_capture()?;
        *self.channel_levels.lock().unwrap() = vec![-60.0; channel_count as usize];
        self.start_capture_internal(input_device, channel_count, sample_rate, bit_depth, None, Some(output_device), RecordingConfig { max_data_bytes: None, drop_mode: false, min_free_bytes: 0 })
    }

    pub fn stop_monitoring(&mut self) -> io::Result<()> {
        log("stop_monitoring called");
        if self.capture_is_recording {
            // Keep recording; just tear down the monitoring output.
            return self.disable_monitor_output();
        }
        self.stop_capture()
    }

    pub fn is_monitoring(&mut self) -> bool {
        match &mut self.capture_aplay {
            None => false,
            Some(p) => match p.try_wait() {
                Ok(None) => true,
                _ => {
                    self.capture_aplay = None;
                    if let Ok(mut sink) = self.capture_monitor_sink.lock() {
                        *sink = None;
                    }
                    false
                }
            },
        }
    }

    /// Start an aplay process and wire its stdin into the running capture thread.
    fn enable_monitor_output(&mut self, output_device: &str, channel_count: u32, sample_rate: u32, bit_depth: u16) -> io::Result<()> {
        // Tear down any previous aplay first.
        self.disable_monitor_output()?;

        let mut aplay = Command::new("aplay")
            .arg("-D")
            .arg(output_device)
            .arg("-c")
            .arg(channel_count.to_string())
            .arg("-r")
            .arg(sample_rate.to_string())
            .arg("-f")
            .arg(alsa_format(bit_depth))
            .arg("-t")
            .arg("raw")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(log_stdio())
            .spawn()
            .map_err(|e| {
                log(&format!("aplay spawn failed: {}", e));
                e
            })?;

        let stdin = aplay
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("aplay stdin missing"))?;

        *self.capture_monitor_sink.lock().unwrap() = Some(stdin);
        self.capture_aplay = Some(aplay);
        log("monitor output enabled on running capture");
        Ok(())
    }

    /// Remove the monitoring output without stopping the capture thread.
    fn disable_monitor_output(&mut self) -> io::Result<()> {
        *self.capture_monitor_sink.lock().unwrap() = None;
        if let Some(mut p) = self.capture_aplay.take() {
            let _ = p.kill();
            let _ = p.wait();
        }
        log("monitor output disabled");
        Ok(())
    }

    /// Stop all capture activity (arecord + aplay + thread). Blocks until clean.
    fn stop_capture(&mut self) -> io::Result<()> {
        // Kill aplay first so the thread unblocks from any blocked write_all.
        if let Some(mut p) = self.capture_aplay.take() {
            let _ = p.kill();
            let _ = p.wait();
        }
        *self.capture_monitor_sink.lock().unwrap() = None;
        // SIGTERM arecord — causes it to flush and close stdout, giving the thread a clean EOF.
        if let Some(mut process) = self.capture_arecord.take() {
            #[cfg(unix)]
            {
                // Send SIGTERM for graceful shutdown (flush buffers).
                unsafe {
                    libc::kill(process.id() as libc::pid_t, libc::SIGTERM);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = process.kill();
            }
            let _ = process.wait();
        }
        // Join thread — if recording, this guarantees WAV header is finalised on disk.
        if let Some(handle) = self.capture_thread.take() {
            let _ = handle.join();
        }
        self.capture_is_recording = false;
        Ok(())
    }

    /// Spawn arecord and a processing thread.
    /// `wav_path` — Some to write a WAV recording.
    /// `monitor_output` — Some(device) to also start aplay and route audio to it.
    #[allow(clippy::too_many_arguments)]
    fn start_capture_internal(
        &mut self,
        input_device: &str,
        channel_count: u32,
        sample_rate: u32,
        bit_depth: u16,
        wav_path: Option<PathBuf>,
        monitor_output: Option<&str>,
        rec_cfg: RecordingConfig,
    ) -> io::Result<()> {
        let RecordingConfig { max_data_bytes, drop_mode, min_free_bytes } = rec_cfg;
        let mut arecord = Command::new("arecord")
            .arg("-D")
            .arg(input_device)
            .arg("-c")
            .arg(channel_count.to_string())
            .arg("-r")
            .arg(sample_rate.to_string())
            .arg("-f")
            .arg(alsa_format(bit_depth))
            .arg("-t")
            .arg("raw")
            .arg("--buffer-size=65536")
            .stdout(Stdio::piped())
            .stderr(log_stdio())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| {
                log(&format!("arecord spawn failed: {}", e));
                e
            })?;

        log("arecord spawned");

        let stdout = arecord
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("arecord stdout missing"))?;

        if let Some(output_device) = monitor_output {
            let mut aplay = match Command::new("aplay")
                .arg("-D")
                .arg(output_device)
                .arg("-c")
                .arg(channel_count.to_string())
                .arg("-r")
                .arg(sample_rate.to_string())
                .arg("-f")
                .arg(alsa_format(bit_depth))
                .arg("-t")
                .arg("raw")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(log_stdio())
                .spawn()
            {
                Ok(p) => {
                    log("aplay spawned");
                    p
                }
                Err(e) => {
                    log(&format!("aplay spawn failed: {}", e));
                    let _ = arecord.kill();
                    let _ = arecord.wait();
                    return Err(e);
                }
            };

            let stdin = aplay
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("aplay stdin missing"))?;
            *self.capture_monitor_sink.lock().unwrap() = Some(stdin);
            self.capture_aplay = Some(aplay);
        } else {
            *self.capture_monitor_sink.lock().unwrap() = None;
        }

        let levels = Arc::clone(&self.channel_levels);
        let monitor_sink = Arc::clone(&self.capture_monitor_sink);
        let recording_bytes = Arc::clone(&self.capture_recording_bytes);
        *recording_bytes.lock().unwrap() = 0;
        let ch = channel_count as usize;

        let handle = thread::spawn(move || {
            capture_and_analyse(stdout, wav_path, monitor_sink, ch, sample_rate, bit_depth, levels, max_data_bytes, drop_mode, recording_bytes, min_free_bytes);
        });

        self.capture_arecord = Some(arecord);
        self.capture_thread = Some(handle);
        Ok(())
    }

    pub fn get_raw_levels(&self) -> Vec<f32> {
        self.channel_levels.lock().unwrap().clone()
    }

    pub fn get_channel_levels(&self) -> Vec<f32> {
        let raw_levels = self.channel_levels.lock().unwrap().clone();

        // Apply volume scaling to the levels
        // Volume adjustment in dB = 20 * log10(volume / 100)
        let volume_db = if self.current_volume > 0 {
            20.0 * (self.current_volume as f32 / 100.0).log10()
        } else {
            -60.0 // Muted
        };

        // Apply volume adjustment to each channel
        raw_levels
            .iter()
            .map(|&level| (level + volume_db).clamp(-60.0, 0.0))
            .collect()
    }
}

/// Reads raw S32_LE PCM from `reader` (arecord stdout).
/// - If `wav_path` is Some, writes a WAV file (header finalised on EOF).
/// - If `monitor_sink` contains Some(stdin), routes audio to aplay; clears on broken pipe.
/// - Updates `levels` with per-channel RMS in dB.
fn free_bytes_on_path(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    let dir = path.parent().unwrap_or(path);
    let cpath = CString::new(dir.to_string_lossy().as_bytes()).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(cpath.as_ptr(), &mut stat) == 0 {
            Some(stat.f_bavail * stat.f_frsize)
        } else {
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_and_analyse<R: Read>(
    mut reader: R,
    wav_path: Option<PathBuf>,
    monitor_sink: Arc<Mutex<Option<ChildStdin>>>,
    channels: usize,
    sample_rate: u32,
    bit_depth: u16,
    levels: Arc<Mutex<Vec<f32>>>,
    max_data_bytes: Option<u64>,
    drop_mode: bool,
    recording_bytes: Arc<Mutex<u64>>,
    min_free_bytes: u64,
) {
    let bps = bytes_per_sample(bit_depth);
    let frame_size = channels * bps;
    let byte_rate = sample_rate * channels as u32 * bps as u32;
    let block_align = channels as u16 * bps as u16;

    // Header is always 80 bytes. For data < 4 GiB we write standard WAV + JUNK pad;
    // for data >= 4 GiB we write RF64 with a ds64 chunk. Both are exactly 80 bytes
    // so the PCM data always starts at offset 80.
    const HEADER_SIZE: u64 = 80;
    const MAX_WAV_DATA: u64 = u32::MAX as u64 - 36;

    let write_header = |f: &mut File, data_bytes: u64| -> io::Result<()> {
        f.seek(SeekFrom::Start(0))?;
        let sample_count = data_bytes / bps as u64;
        if data_bytes <= MAX_WAV_DATA {
            // Standard WAV with JUNK chunk to fill the ds64 space (36 bytes).
            let riff_size = (data_bytes + (HEADER_SIZE - 8)) as u32;
            f.write_all(b"RIFF")?;
            f.write_all(&riff_size.to_le_bytes())?;
            f.write_all(b"WAVE")?;
            // JUNK chunk: 4 (id) + 4 (size) + 28 (payload) = 36 bytes
            f.write_all(b"JUNK")?;
            f.write_all(&28u32.to_le_bytes())?;
            f.write_all(&[0u8; 28])?;
            // fmt chunk
            f.write_all(b"fmt ")?;
            f.write_all(&16u32.to_le_bytes())?;
            f.write_all(&1u16.to_le_bytes())?; // PCM
            f.write_all(&(channels as u16).to_le_bytes())?;
            f.write_all(&sample_rate.to_le_bytes())?;
            f.write_all(&byte_rate.to_le_bytes())?;
            f.write_all(&block_align.to_le_bytes())?;
            f.write_all(&bit_depth.to_le_bytes())?;
            // data chunk
            f.write_all(b"data")?;
            f.write_all(&(data_bytes as u32).to_le_bytes())?;
        } else {
            // RF64 header with ds64 chunk for >4 GiB recordings.
            let riff_size = 0xFFFFFFFFu32;
            f.write_all(b"RF64")?;
            f.write_all(&riff_size.to_le_bytes())?;
            f.write_all(b"WAVE")?;
            // ds64 chunk: 4 (id) + 4 (size) + 28 (payload) = 36 bytes
            f.write_all(b"ds64")?;
            f.write_all(&28u32.to_le_bytes())?;
            let rf64_riff_size = data_bytes + (HEADER_SIZE - 8);
            f.write_all(&rf64_riff_size.to_le_bytes())?; // riffSize (u64)
            f.write_all(&data_bytes.to_le_bytes())?; // dataSize (u64)
            f.write_all(&sample_count.to_le_bytes())?; // sampleCount (u64)
            f.write_all(&0u32.to_le_bytes())?; // tableLength
            // fmt chunk
            f.write_all(b"fmt ")?;
            f.write_all(&16u32.to_le_bytes())?;
            f.write_all(&1u16.to_le_bytes())?; // PCM
            f.write_all(&(channels as u16).to_le_bytes())?;
            f.write_all(&sample_rate.to_le_bytes())?;
            f.write_all(&byte_rate.to_le_bytes())?;
            f.write_all(&block_align.to_le_bytes())?;
            f.write_all(&bit_depth.to_le_bytes())?;
            // data chunk — size sentinel for RF64
            f.write_all(b"data")?;
            f.write_all(&0xFFFFFFFFu32.to_le_bytes())?;
        }
        Ok(())
    };

    // Max data size in bytes (after header). Align to frame boundary.
    // max_data is mutable — disk space guard can lower it dynamically.
    let mut max_data: Option<u64> = max_data_bytes.map(|m| {
        let m = m.saturating_sub(HEADER_SIZE);
        (m / frame_size as u64) * frame_size as u64 // align to frame
    });

    // Open WAV file if recording — write placeholder header.
    // Must be read+write so drop mode can read back data during linearization.
    let mut wav: Option<(File, u64)> = wav_path.as_ref().and_then(|path| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .ok()
            .and_then(|mut f| write_header(&mut f, 0).ok().map(|_| (f, 0u64)))
    });

    // Circular buffer state for drop mode.
    // `total` = total data bytes written so far (may exceed max in drop mode due to wrapping).
    // `write_offset` = current file offset for the next write.
    // `wrapped` = whether we have wrapped around at least once.
    let mut write_offset: u64 = HEADER_SIZE;
    let mut wrapped = false;
    // `logical_bytes` = how many bytes of real audio are in the file (capped at max_data).
    let mut logical_bytes: u64 = 0;

    let window_frames: usize = (sample_rate / 10) as usize; // 100 ms
    let mut ch_bufs: Vec<Vec<f32>> = vec![vec![]; channels];
    let mut cur_levels = vec![-60.0f32; channels];
    let buf_size = frame_size * 4096;
    let mut buf = vec![0u8; buf_size];
    let mut stopped_by_limit = false;
    let mut bytes_since_free_check: u64 = 0;
    const FREE_CHECK_INTERVAL: u64 = 4 * 1024 * 1024; // check every 4 MB written

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // Write to WAV file.
                if let Some((ref mut file, ref mut _total)) = wav {
                    let data = &buf[..n];
                    let mut written = 0usize;

                    while written < data.len() {
                        let chunk = &data[written..];

                        if let Some(max) = max_data {
                            if !drop_mode {
                                // Stop mode: check if we'd exceed the limit.
                                let remaining = max.saturating_sub(logical_bytes);
                                if remaining == 0 {
                                    stopped_by_limit = true;
                                    break;
                                }
                                let to_write = chunk.len().min(remaining as usize);
                                if file.write_all(&chunk[..to_write]).is_ok() {
                                    logical_bytes += to_write as u64;
                                    write_offset += to_write as u64;
                                }
                                written += to_write;
                            } else {
                                // Drop mode: circular write within the data region.
                                let data_end = HEADER_SIZE + max;
                                let space_to_end = (data_end - write_offset) as usize;
                                let to_write = chunk.len().min(space_to_end);

                                let _ = file.seek(SeekFrom::Start(write_offset));
                                if file.write_all(&chunk[..to_write]).is_ok() {
                                    write_offset += to_write as u64;
                                    logical_bytes = (logical_bytes + to_write as u64).min(max);
                                    written += to_write;
                                } else {
                                    break;
                                }

                                // Wrap around if we hit the end of the data region.
                                if write_offset >= data_end {
                                    write_offset = HEADER_SIZE;
                                    wrapped = true;
                                }
                            }
                        } else {
                            // Unlimited: simple sequential write.
                            if file.write_all(chunk).is_ok() {
                                logical_bytes += chunk.len() as u64;
                                write_offset += chunk.len() as u64;
                            }
                            written = data.len();
                        }
                    }

                    // Update shared byte counter for UI.
                    if let Ok(mut g) = recording_bytes.lock() {
                        *g = logical_bytes;
                    }

                    if stopped_by_limit {
                        break;
                    }

                    // Disk space guard: check periodically.
                    // In drop mode, locks the current file size as the circular cap.
                    // In stop mode, stops the recording.
                    if min_free_bytes > 0 {
                        bytes_since_free_check += n as u64;
                        if bytes_since_free_check >= FREE_CHECK_INTERVAL {
                            bytes_since_free_check = 0;
                            if let Some(path) = &wav_path {
                                if let Some(free) = free_bytes_on_path(path) {
                                    if free < min_free_bytes {
                                        if !drop_mode {
                                            log(&format!(
                                                "capture_and_analyse: disk space low ({} bytes free), stopping",
                                                free
                                            ));
                                            stopped_by_limit = true;
                                            break;
                                        } else if !wrapped {
                                            // Lock current size as the circular buffer cap.
                                            let aligned = (logical_bytes / frame_size as u64) * frame_size as u64;
                                            if max_data.is_none_or(|m| aligned < m) {
                                                log(&format!(
                                                    "capture_and_analyse: disk space low ({} bytes free), capping at {} bytes and looping",
                                                    free, aligned
                                                ));
                                                max_data = Some(aligned);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Write to monitoring output; clear on broken pipe.
                {
                    let mut sink = monitor_sink.lock().unwrap();
                    if let Some(ref mut w) = *sink {
                        if w.write_all(&buf[..n]).is_err() {
                            *sink = None::<ChildStdin>;
                        }
                    }
                }
                // Level analysis: decode samples.
                let num_frames = n / frame_size;
                for i in 0..num_frames {
                    #[allow(clippy::needless_range_loop)]
                    for ch in 0..channels {
                        let off = i * frame_size + ch * bps;
                        if off + bps <= n {
                            let sample = match bit_depth {
                                16 => {
                                    let s = i16::from_le_bytes([buf[off], buf[off + 1]]);
                                    s as f32 / i16::MAX as f32
                                }
                                24 => {
                                    // S24_3LE: 3 bytes, sign-extend to i32
                                    let s = ((buf[off] as i32)
                                        | ((buf[off + 1] as i32) << 8)
                                        | ((buf[off + 2] as i32) << 16))
                                        << 8 >> 8; // sign-extend
                                    s as f32 / 0x7FFFFF as f32
                                }
                                _ => {
                                    let s = i32::from_le_bytes([
                                        buf[off],
                                        buf[off + 1],
                                        buf[off + 2],
                                        buf[off + 3],
                                    ]);
                                    s as f32 / i32::MAX as f32
                                }
                            };
                            ch_bufs[ch].push(sample);
                        }
                    }
                }
                let mut updated = false;
                #[allow(clippy::needless_range_loop)]
                for ch in 0..channels {
                    if ch_bufs[ch].len() >= window_frames {
                        let rms = (ch_bufs[ch].iter().map(|&s| s * s).sum::<f32>()
                            / ch_bufs[ch].len() as f32)
                            .sqrt();
                        let db = if rms > 0.0 { 20.0 * rms.log10() } else { -60.0 };
                        cur_levels[ch] = db.clamp(-60.0, 0.0);
                        ch_bufs[ch].clear();
                        updated = true;
                    }
                }
                if updated {
                    if let Ok(mut g) = levels.lock() {
                        *g = cur_levels.clone();
                    }
                }
            }
            Err(_) => break,
        }
    }

    // Finalise: if drop mode wrapped, linearize the circular buffer.
    if let Some((mut file, _)) = wav {
        if drop_mode && wrapped {
            if let Some(max) = max_data {
                // The write_offset is where the oldest data starts (it's the next write position).
                // Data order: [write_offset .. HEADER_SIZE+max] then [HEADER_SIZE .. write_offset]
                let data_end = HEADER_SIZE + max;
                let path = wav_path.as_ref().unwrap();
                let tmp_path = path.with_extension("wav.tmp");

                if let Ok(mut tmp) = File::create(&tmp_path) {
                    // Write header to temp file.
                    let _ = write_header(&mut tmp, logical_bytes);

                    // Copy: oldest data first (from write_offset to end of data region).
                    let mut copy_buf = vec![0u8; 64 * 1024];
                    let mut pos = write_offset;
                    while pos < data_end {
                        let to_read = copy_buf.len().min((data_end - pos) as usize);
                        let _ = file.seek(SeekFrom::Start(pos));
                        if let Ok(n) = file.read(&mut copy_buf[..to_read]) {
                            if n == 0 { break; }
                            let _ = tmp.write_all(&copy_buf[..n]);
                            pos += n as u64;
                        } else {
                            break;
                        }
                    }

                    // Copy: newest data (from HEADER_SIZE to write_offset).
                    pos = HEADER_SIZE;
                    while pos < write_offset {
                        let to_read = copy_buf.len().min((write_offset - pos) as usize);
                        let _ = file.seek(SeekFrom::Start(pos));
                        if let Ok(n) = file.read(&mut copy_buf[..to_read]) {
                            if n == 0 { break; }
                            let _ = tmp.write_all(&copy_buf[..n]);
                            pos += n as u64;
                        } else {
                            break;
                        }
                    }

                    drop(tmp);
                    drop(file);
                    let _ = std::fs::rename(&tmp_path, path);
                    log(&format!(
                        "capture_and_analyse: linearized circular buffer, {} bytes",
                        logical_bytes
                    ));
                    return;
                }
            }
        }

        // Normal finalize (no wrap, or unlimited, or stop mode).
        let _ = write_header(&mut file, logical_bytes);
        let fmt = if logical_bytes > MAX_WAV_DATA {
            "RF64"
        } else {
            "WAV"
        };
        log(&format!(
            "capture_and_analyse: {} bytes written as {}{}",
            logical_bytes,
            fmt,
            if stopped_by_limit {
                " (stopped at limit)"
            } else {
                ""
            }
        ));
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.stop_capture();
        let _ = self.stop();
    }
}

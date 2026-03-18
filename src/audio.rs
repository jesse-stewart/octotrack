use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

#[cfg(unix)]
extern crate libc;

/// WAV header is always exactly 80 bytes (standard WAV with JUNK pad, or RF64 with ds64).
const HEADER_SIZE: u64 = 80;
/// Maximum data bytes that fit in a standard WAV (u32 size field minus the 36-byte overhead).
const MAX_WAV_DATA: u64 = u32::MAX as u64 - 36;

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

/// Returns `{base_stem}_{index:03}.{ext}` in the same directory as `base`.
fn split_file_path(base: &Path, index: u32) -> PathBuf {
    let stem = base.file_stem().unwrap_or_default().to_string_lossy();
    let ext = base
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wav".to_string());
    let dir = base.parent().unwrap_or(Path::new("."));
    dir.join(format!("{}_{:03}.{}", stem, index, ext))
}

/// Opens (or creates) a WAV file at `path` and writes an 80-byte placeholder header
/// (data length = 0).  The file is opened read+write so drop-mode can seek back later.
/// Returns an error if the file cannot be created or the header cannot be written.
fn open_wav_file(
    path: &Path,
    channels: usize,
    sample_rate: u32,
    bit_depth: u16,
) -> io::Result<File> {
    let bps = bytes_per_sample(bit_depth);
    let byte_rate = sample_rate * channels as u32 * bps as u32;
    let block_align = channels as u16 * bps as u16;
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    // 80-byte placeholder: RIFF/JUNK/fmt/data with data_bytes = 0.
    f.write_all(b"RIFF")?;
    f.write_all(&72u32.to_le_bytes())?; // riff_size = 80 - 8
    f.write_all(b"WAVE")?;
    f.write_all(b"JUNK")?;
    f.write_all(&28u32.to_le_bytes())?;
    f.write_all(&[0u8; 28])?;
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&(channels as u16).to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&bit_depth.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&0u32.to_le_bytes())?;
    Ok(f)
}

/// Repair a WAV file whose header under-reports the actual data size (e.g. after
/// a crash or power loss).  Reads the file size, compares it to the header's
/// claimed data length, and rewrites the header if the file is larger.
fn repair_wav_header(path: &Path) -> io::Result<bool> {
    let meta = std::fs::metadata(path)?;
    let file_size = meta.len();
    if file_size <= HEADER_SIZE {
        return Ok(false);
    }

    let mut f = OpenOptions::new().read(true).write(true).open(path)?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    if &magic != b"RIFF" && &magic != b"RF64" {
        return Ok(false);
    }

    let actual_data = file_size - HEADER_SIZE;

    // Read claimed data size from header.
    let claimed_data = if &magic == b"RF64" {
        // ds64 dataSize is at offset 40 (8 bytes).
        f.seek(SeekFrom::Start(40))?;
        let mut buf = [0u8; 8];
        f.read_exact(&mut buf)?;
        u64::from_le_bytes(buf)
    } else {
        // Standard WAV: data chunk size at offset 76 (4 bytes).
        f.seek(SeekFrom::Start(76))?;
        let mut buf = [0u8; 4];
        f.read_exact(&mut buf)?;
        u32::from_le_bytes(buf) as u64
    };

    if actual_data <= claimed_data {
        return Ok(false);
    }

    // Read fmt chunk fields (channels, sample_rate, byte_rate, block_align, bit_depth)
    // starting at offset 58 (after RIFF/WAVE + JUNK/ds64 + "fmt " + fmt_size).
    f.seek(SeekFrom::Start(58))?;
    let mut hdr = [0u8; 14];
    f.read_exact(&mut hdr)?;
    let channels = u16::from_le_bytes([hdr[0], hdr[1]]) as u64;
    let sample_rate = u32::from_le_bytes([hdr[2], hdr[3], hdr[4], hdr[5]]);
    let byte_rate = u32::from_le_bytes([hdr[6], hdr[7], hdr[8], hdr[9]]);
    let block_align = u16::from_le_bytes([hdr[10], hdr[11]]);
    let bit_depth = u16::from_le_bytes([hdr[12], hdr[13]]);
    let bps = bytes_per_sample(bit_depth) as u64;

    // Align to frame boundary.
    let frame_size = channels * bps;
    if frame_size == 0 {
        return Ok(false);
    }
    let aligned_data = (actual_data / frame_size) * frame_size;
    let sample_count = aligned_data / bps;

    // Rewrite the full 80-byte header.
    f.seek(SeekFrom::Start(0))?;
    if aligned_data <= MAX_WAV_DATA {
        let riff_size = (aligned_data + (HEADER_SIZE - 8)) as u32;
        f.write_all(b"RIFF")?;
        f.write_all(&riff_size.to_le_bytes())?;
        f.write_all(b"WAVE")?;
        f.write_all(b"JUNK")?;
        f.write_all(&28u32.to_le_bytes())?;
        f.write_all(&[0u8; 28])?;
    } else {
        f.write_all(b"RF64")?;
        f.write_all(&0xFFFFFFFFu32.to_le_bytes())?;
        f.write_all(b"WAVE")?;
        f.write_all(b"ds64")?;
        f.write_all(&28u32.to_le_bytes())?;
        let rf64_riff_size = aligned_data + (HEADER_SIZE - 8);
        f.write_all(&rf64_riff_size.to_le_bytes())?;
        f.write_all(&aligned_data.to_le_bytes())?;
        f.write_all(&sample_count.to_le_bytes())?;
        f.write_all(&0u32.to_le_bytes())?;
    }
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&(channels as u16).to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&bit_depth.to_le_bytes())?;
    f.write_all(b"data")?;
    if aligned_data <= MAX_WAV_DATA {
        f.write_all(&(aligned_data as u32).to_le_bytes())?;
    } else {
        f.write_all(&0xFFFFFFFFu32.to_le_bytes())?;
    }
    f.flush()?;

    log(&format!(
        "repair_wav_header: {:?} repaired: claimed={} actual={}",
        path, claimed_data, aligned_data
    ));
    Ok(true)
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
    /// Split recording into multiple files of this size. None = no splitting.
    /// Combined with drop_mode: false → keep all files; drop_mode: true → delete the previous
    /// file on each roll (dashcam style). Files are named `{stem}_001.wav`, `{stem}_002.wav`, …
    pub split_size_bytes: Option<u64>,
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

            // Repair any WAV files with stale headers (e.g. from crash/power loss).
            for f in &audio_files {
                if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
                    let _ = repair_wav_header(f);
                }
            }

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
        // Repair WAV header if it was left stale by a crash/power loss.
        if file_path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
        {
            let _ = repair_wav_header(file_path);
        }

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

        // Repair any WAV files with stale headers (e.g. from crash/power loss).
        for f in &audio_files {
            if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
                let _ = repair_wav_header(f);
            }
        }

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
        self.start_capture_internal(
            input_device,
            channel_count,
            sample_rate,
            bit_depth,
            Some(output_path.to_path_buf()),
            None,
            rec_cfg,
        )?;
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
            return self.enable_monitor_output(
                output_device,
                channel_count,
                sample_rate,
                bit_depth,
            );
        }

        // No active capture — start a monitoring-only session.
        self.stop_capture()?;
        *self.channel_levels.lock().unwrap() = vec![-60.0; channel_count as usize];
        self.start_capture_internal(
            input_device,
            channel_count,
            sample_rate,
            bit_depth,
            None,
            Some(output_device),
            RecordingConfig {
                max_data_bytes: None,
                drop_mode: false,
                min_free_bytes: 0,
                split_size_bytes: None,
            },
        )
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
    fn enable_monitor_output(
        &mut self,
        output_device: &str,
        channel_count: u32,
        sample_rate: u32,
        bit_depth: u16,
    ) -> io::Result<()> {
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
        let RecordingConfig {
            max_data_bytes,
            drop_mode,
            min_free_bytes,
            split_size_bytes,
        } = rec_cfg;
        let ch = channel_count as usize;

        // Open WAV file before spawning any processes so failures surface immediately.
        // If splitting, the first file gets a _001 suffix; wav_path remains the base for
        // generating subsequent split filenames inside the capture thread.
        let first_path = match &wav_path {
            Some(p) if split_size_bytes.is_some() => Some(split_file_path(p, 1)),
            _ => wav_path.clone(),
        };
        let wav_file: Option<File> = match &first_path {
            None => None,
            Some(path) => Some(open_wav_file(path, ch, sample_rate, bit_depth)?),
        };

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

        let handle = thread::spawn(move || {
            capture_and_analyse(
                stdout,
                wav_file,
                wav_path,
                monitor_sink,
                ch,
                sample_rate,
                bit_depth,
                levels,
                max_data_bytes,
                drop_mode,
                recording_bytes,
                min_free_bytes,
                split_size_bytes,
            );
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
    wav_file: Option<File>,
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
    split_size_bytes: Option<u64>,
) {
    let bps = bytes_per_sample(bit_depth);
    let frame_size = channels * bps;
    let byte_rate = sample_rate * channels as u32 * bps as u32;
    let block_align = channels as u16 * bps as u16;

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

    // WAV file was already opened (and placeholder header written) by the caller.
    let mut wav: Option<(File, u64)> = wav_file.map(|f| (f, 0u64));

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
    // Carry buffer for level analysis: holds bytes from the end of the last read that didn't
    // form a complete frame.  A file-split causes extra I/O in this thread, delaying reads;
    // when we resume the pipe may have a non-frame-aligned byte count queued up.  Without the
    // carry buffer buf[0] would be treated as channel 0 even when it's mid-frame, making
    // channels 1-4 appear to swap with channels 5-8 until the drift self-corrects.
    let mut level_carry: Vec<u8> = Vec::with_capacity(frame_size);
    let buf_size = frame_size * 4096;
    let mut buf = vec![0u8; buf_size];
    let mut stopped_by_limit = false;
    let mut bytes_since_free_check: u64 = 0;
    const FREE_CHECK_INTERVAL: u64 = 4 * 1024 * 1024; // check every 4 MB written

    // Periodic header flush for crash safety (~10 seconds of audio between flushes).
    // Disabled for circular drop mode (header can only be finalized after linearization).
    let periodic_flush = !(drop_mode && split_size_bytes.is_none());
    let flush_interval_bytes: u64 = byte_rate as u64 * 10;
    let mut bytes_since_flush: u64 = 0;
    // total_bytes accumulates across all split files; used for the UI counter.
    // logical_bytes tracks only the current file (for header finalisation and drop-mode).
    let mut total_bytes: u64 = 0;
    let mut split_file_index: u32 = 1; // current split file number (1-based)
                                       // In drop+split mode: queue of completed file paths for oldest-first eviction.
                                       // We keep at most (max_data_bytes / split_size_bytes) files on disk at once.
    let max_split_files: Option<usize> = if drop_mode {
        split_size_bytes
            .zip(max_data_bytes)
            .map(|(s, m)| (m / s).max(1) as usize)
    } else {
        None
    };
    let mut completed_split_paths: std::collections::VecDeque<PathBuf> =
        std::collections::VecDeque::new();

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // Write to WAV file.
                if wav.is_some() {
                    let data = &buf[..n];
                    let mut written = 0usize;

                    while written < data.len() {
                        // Roll to the next split file when the current one is full.
                        // (Splitting is only supported in non-drop mode.)
                        if let Some(split) = split_size_bytes {
                            if logical_bytes >= split {
                                let old_path =
                                    split_file_path(wav_path.as_ref().unwrap(), split_file_index);
                                if let Some((ref mut f, _)) = wav {
                                    let _ = write_header(f, logical_bytes);
                                }
                                // In drop mode, maintain a rolling window of completed files.
                                // Evict the oldest when the window is full.
                                if drop_mode {
                                    completed_split_paths.push_back(old_path);
                                    if let Some(max_f) = max_split_files {
                                        while completed_split_paths.len() >= max_f {
                                            if let Some(oldest) = completed_split_paths.pop_front()
                                            {
                                                if let Err(e) = std::fs::remove_file(&oldest) {
                                                    log(&format!(
                                                        "capture_and_analyse: failed to remove old split file {:?}: {}",
                                                        oldest, e
                                                    ));
                                                } else {
                                                    log(&format!(
                                                        "capture_and_analyse: removed old split file {:?}",
                                                        oldest
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                                split_file_index += 1;
                                let next_path =
                                    split_file_path(wav_path.as_ref().unwrap(), split_file_index);
                                log(&format!(
                                    "capture_and_analyse: opening split file {:?}",
                                    next_path
                                ));
                                wav = open_wav_file(&next_path, channels, sample_rate, bit_depth)
                                    .ok()
                                    .map(|f| (f, 0u64));
                                logical_bytes = 0;
                                write_offset = HEADER_SIZE;
                                bytes_since_flush = 0;
                                if wav.is_none() {
                                    log(&format!(
                                        "capture_and_analyse: failed to open split file {:?}, stopping",
                                        next_path
                                    ));
                                    stopped_by_limit = true;
                                    break;
                                }
                            }
                        }

                        let chunk = &data[written..];
                        let (file, _) = wav.as_mut().unwrap();

                        if drop_mode && split_size_bytes.is_none() {
                            // Circular buffer mode: write within a fixed data region.
                            let max = max_data.unwrap_or(u64::MAX);
                            let data_end = HEADER_SIZE + max;
                            let space_to_end = (data_end - write_offset) as usize;
                            let to_write = chunk.len().min(space_to_end);

                            let _ = file.seek(SeekFrom::Start(write_offset));
                            if file.write_all(&chunk[..to_write]).is_ok() {
                                write_offset += to_write as u64;
                                logical_bytes = (logical_bytes + to_write as u64).min(max);
                                total_bytes = logical_bytes;
                                written += to_write;
                            } else {
                                break;
                            }

                            if write_offset >= data_end {
                                write_offset = HEADER_SIZE;
                                wrapped = true;
                            }
                        } else if !drop_mode {
                            if let Some(max) = max_data {
                                // Stop mode: limit is total across all split files.
                                let remaining = max.saturating_sub(total_bytes);
                                if remaining == 0 {
                                    stopped_by_limit = true;
                                    break;
                                }
                                let to_write = chunk.len().min(remaining as usize);
                                if file.write_all(&chunk[..to_write]).is_ok() {
                                    logical_bytes += to_write as u64;
                                    total_bytes += to_write as u64;
                                    write_offset += to_write as u64;
                                }
                                written += to_write;
                            } else {
                                // Unlimited, with optional per-file split limit.
                                let to_write = if let Some(split) = split_size_bytes {
                                    chunk
                                        .len()
                                        .min(split.saturating_sub(logical_bytes) as usize)
                                } else {
                                    chunk.len()
                                };
                                if file.write_all(&chunk[..to_write]).is_ok() {
                                    logical_bytes += to_write as u64;
                                    total_bytes += to_write as u64;
                                    write_offset += to_write as u64;
                                }
                                written += to_write;
                            }
                        } else {
                            // drop_mode + split_size_bytes: sequential write; rolling and
                            // deletion of the old file are handled at the top of this loop.
                            let to_write = chunk
                                .len()
                                .min(split_size_bytes.unwrap().saturating_sub(logical_bytes)
                                    as usize);
                            if file.write_all(&chunk[..to_write]).is_ok() {
                                logical_bytes += to_write as u64;
                                total_bytes += to_write as u64;
                                write_offset += to_write as u64;
                            }
                            written += to_write;
                        }
                    }

                    // Update shared byte counter for UI (total across all split files).
                    if let Ok(mut g) = recording_bytes.lock() {
                        *g = total_bytes;
                    }

                    if stopped_by_limit {
                        break;
                    }

                    // Periodic header flush: rewrite header with current data size so
                    // a crash/power loss leaves a valid (if slightly short) WAV file.
                    if periodic_flush {
                        bytes_since_flush += n as u64;
                        if bytes_since_flush >= flush_interval_bytes {
                            bytes_since_flush = 0;
                            if let Some((ref mut f, _)) = wav {
                                if write_header(f, logical_bytes).is_ok() {
                                    let _ = f.seek(SeekFrom::Start(write_offset));
                                    #[cfg(unix)]
                                    {
                                        use std::os::unix::io::AsRawFd;
                                        unsafe { libc::fsync(f.as_raw_fd()) };
                                    }
                                }
                            }
                        }
                    }

                    // Disk space guard: check periodically.
                    // In circular-buffer drop mode, locks the current file size as the cap.
                    // In all other modes (stop, split, split+drop), stops the recording.
                    if min_free_bytes > 0 {
                        bytes_since_free_check += n as u64;
                        if bytes_since_free_check >= FREE_CHECK_INTERVAL {
                            bytes_since_free_check = 0;
                            if let Some(path) = &wav_path {
                                if let Some(free) = free_bytes_on_path(path) {
                                    if free < min_free_bytes {
                                        let is_circular = drop_mode && split_size_bytes.is_none();
                                        if !is_circular {
                                            log(&format!(
                                                "capture_and_analyse: disk space low ({} bytes free), stopping",
                                                free
                                            ));
                                            stopped_by_limit = true;
                                            break;
                                        } else if !wrapped {
                                            // Lock current size as the circular buffer cap.
                                            let aligned = (logical_bytes / frame_size as u64)
                                                * frame_size as u64;
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
                // Prepend any carry bytes from the previous read so that we always start
                // decoding at a frame boundary even when the OS delivers a non-frame-aligned
                // byte count (e.g. after the I/O pause caused by a file split).
                level_carry.extend_from_slice(&buf[..n]);
                let num_frames = level_carry.len() / frame_size;
                for i in 0..num_frames {
                    #[allow(clippy::needless_range_loop)]
                    for ch in 0..channels {
                        let off = i * frame_size + ch * bps;
                        let sample = match bit_depth {
                            16 => {
                                let s =
                                    i16::from_le_bytes([level_carry[off], level_carry[off + 1]]);
                                s as f32 / i16::MAX as f32
                            }
                            24 => {
                                // S24_3LE: 3 bytes, sign-extend to i32
                                let s = ((level_carry[off] as i32)
                                    | ((level_carry[off + 1] as i32) << 8)
                                    | ((level_carry[off + 2] as i32) << 16))
                                    << 8
                                    >> 8; // sign-extend
                                s as f32 / 0x7FFFFF as f32
                            }
                            _ => {
                                let s = i32::from_le_bytes([
                                    level_carry[off],
                                    level_carry[off + 1],
                                    level_carry[off + 2],
                                    level_carry[off + 3],
                                ]);
                                s as f32 / i32::MAX as f32
                            }
                        };
                        ch_bufs[ch].push(sample);
                    }
                }
                let consumed = num_frames * frame_size;
                level_carry.drain(..consumed);
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
                            if n == 0 {
                                break;
                            }
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
                            if n == 0 {
                                break;
                            }
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
        let files_note = if split_file_index > 1 {
            format!(" ({} files)", split_file_index)
        } else {
            String::new()
        };
        log(&format!(
            "capture_and_analyse: {} bytes written as {}{}{}",
            total_bytes,
            fmt,
            files_note,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a valid WAV file with octotrack's 80-byte header and fake PCM data.
    fn create_test_wav(
        path: &Path,
        channels: usize,
        sample_rate: u32,
        bit_depth: u16,
        data_bytes: usize,
    ) {
        let mut f = open_wav_file(path, channels, sample_rate, bit_depth).unwrap();
        // Write fake PCM data (zeros are valid silent audio).
        let data = vec![0u8; data_bytes];
        f.write_all(&data).unwrap();
        // Finalize header with correct data size.
        f.seek(SeekFrom::Start(0)).unwrap();
        let bps = bytes_per_sample(bit_depth);
        let byte_rate = sample_rate * channels as u32 * bps as u32;
        let block_align = channels as u16 * bps as u16;
        let data_bytes = data_bytes as u64;
        if data_bytes <= MAX_WAV_DATA {
            let riff_size = (data_bytes + (HEADER_SIZE - 8)) as u32;
            f.write_all(b"RIFF").unwrap();
            f.write_all(&riff_size.to_le_bytes()).unwrap();
            f.write_all(b"WAVE").unwrap();
            f.write_all(b"JUNK").unwrap();
            f.write_all(&28u32.to_le_bytes()).unwrap();
            f.write_all(&[0u8; 28]).unwrap();
        }
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap();
        f.write_all(&(channels as u16).to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        f.write_all(&block_align.to_le_bytes()).unwrap();
        f.write_all(&bit_depth.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&(data_bytes as u32).to_le_bytes()).unwrap();
        f.flush().unwrap();
    }

    /// Read the data chunk size from offset 76 of an 80-byte octotrack WAV header.
    fn read_data_size(path: &Path) -> u32 {
        let mut f = File::open(path).unwrap();
        f.seek(SeekFrom::Start(76)).unwrap();
        let mut buf = [0u8; 4];
        f.read_exact(&mut buf).unwrap();
        u32::from_le_bytes(buf)
    }

    /// Zero out the data chunk size at offset 76 (simulates a crash).
    fn corrupt_data_size(path: &Path) {
        let mut f = OpenOptions::new().write(true).open(path).unwrap();
        f.seek(SeekFrom::Start(76)).unwrap();
        f.write_all(&0u32.to_le_bytes()).unwrap();
        f.flush().unwrap();
    }

    #[test]
    fn repair_fixes_zeroed_data_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        let data_bytes = 48000 * 2 * 4 * 10; // 10s of stereo 32-bit 48kHz
        create_test_wav(&path, 2, 48000, 32, data_bytes);

        assert_eq!(read_data_size(&path), data_bytes as u32);
        corrupt_data_size(&path);
        assert_eq!(read_data_size(&path), 0);

        let repaired = repair_wav_header(&path).unwrap();
        assert!(repaired);
        assert_eq!(read_data_size(&path), data_bytes as u32);
    }

    #[test]
    fn repair_skips_correct_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("good.wav");
        let data_bytes = 48000 * 2 * 4; // 1s
        create_test_wav(&path, 2, 48000, 32, data_bytes);

        let repaired = repair_wav_header(&path).unwrap();
        assert!(!repaired);
        assert_eq!(read_data_size(&path), data_bytes as u32);
    }

    #[test]
    fn repair_handles_8_channel_32bit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("8ch.wav");
        let data_bytes = 48000 * 8 * 4 * 5; // 5s of 8-channel 32-bit
        create_test_wav(&path, 8, 48000, 32, data_bytes);

        corrupt_data_size(&path);
        let repaired = repair_wav_header(&path).unwrap();
        assert!(repaired);
        assert_eq!(read_data_size(&path), data_bytes as u32);
    }

    #[test]
    fn repair_handles_16bit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("16bit.wav");
        let data_bytes = 48000 * 2 * 2 * 3; // 3s of stereo 16-bit
        create_test_wav(&path, 2, 48000, 16, data_bytes);

        corrupt_data_size(&path);
        let repaired = repair_wav_header(&path).unwrap();
        assert!(repaired);
        assert_eq!(read_data_size(&path), data_bytes as u32);
    }

    #[test]
    fn repair_handles_24bit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("24bit.wav");
        let data_bytes = 48000 * 2 * 3 * 3; // 3s of stereo 24-bit
        create_test_wav(&path, 2, 48000, 24, data_bytes);

        corrupt_data_size(&path);
        let repaired = repair_wav_header(&path).unwrap();
        assert!(repaired);
        assert_eq!(read_data_size(&path), data_bytes as u32);
    }

    #[test]
    fn repair_skips_non_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_a_wav.wav");
        std::fs::write(&path, b"this is not a wav file at all").unwrap();

        let repaired = repair_wav_header(&path).unwrap();
        assert!(!repaired);
    }

    #[test]
    fn repair_skips_tiny_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.wav");
        std::fs::write(&path, &[0u8; 40]).unwrap();

        let repaired = repair_wav_header(&path).unwrap();
        assert!(!repaired);
    }

    // -----------------------------------------------------------------------
    // bytes_per_sample
    // -----------------------------------------------------------------------

    #[test]
    fn bytes_per_sample_values() {
        assert_eq!(bytes_per_sample(16), 2);
        assert_eq!(bytes_per_sample(24), 3);
        assert_eq!(bytes_per_sample(32), 4);
        assert_eq!(bytes_per_sample(0), 4); // default arm
    }

    // -----------------------------------------------------------------------
    // split_file_path
    // -----------------------------------------------------------------------

    #[test]
    fn split_file_path_format() {
        let base = Path::new("/tmp/tracks/REC_123.wav");
        assert_eq!(
            split_file_path(base, 1),
            PathBuf::from("/tmp/tracks/REC_123_001.wav")
        );
        assert_eq!(
            split_file_path(base, 42),
            PathBuf::from("/tmp/tracks/REC_123_042.wav")
        );
    }

    #[test]
    fn split_file_path_no_extension() {
        let base = Path::new("/tmp/tracks/REC_123");
        assert_eq!(
            split_file_path(base, 1),
            PathBuf::from("/tmp/tracks/REC_123_001.wav")
        );
    }

    // -----------------------------------------------------------------------
    // open_wav_file
    // -----------------------------------------------------------------------

    #[test]
    fn open_wav_file_creates_80_byte_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        let f = open_wav_file(&path, 2, 48000, 32).unwrap();
        drop(f);

        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 80);
    }

    #[test]
    fn open_wav_file_header_has_riff_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        let f = open_wav_file(&path, 2, 48000, 32).unwrap();
        drop(f);

        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[12..16], b"JUNK");
        assert_eq!(&data[48..52], b"fmt ");
        assert_eq!(&data[72..76], b"data");
    }

    #[test]
    fn open_wav_file_data_size_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        let f = open_wav_file(&path, 8, 192000, 32).unwrap();
        drop(f);

        assert_eq!(read_data_size(&path), 0);
    }

    #[test]
    fn open_wav_file_stores_correct_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        let f = open_wav_file(&path, 8, 96000, 24).unwrap();
        drop(f);

        let data = std::fs::read(&path).unwrap();
        // channels at offset 58
        let channels = u16::from_le_bytes([data[58], data[59]]);
        assert_eq!(channels, 8);
        // sample rate at offset 60
        let sr = u32::from_le_bytes([data[60], data[61], data[62], data[63]]);
        assert_eq!(sr, 96000);
        // bit depth at offset 70
        let bd = u16::from_le_bytes([data[70], data[71]]);
        assert_eq!(bd, 24);
    }

    // -----------------------------------------------------------------------
    // capture_and_analyse — recording engine tests
    // -----------------------------------------------------------------------

    /// Helper to run capture_and_analyse with synthetic PCM data.
    /// Returns the recording_bytes counter value after capture completes.
    fn run_capture(
        pcm_data: &[u8],
        wav_path: &Path,
        channels: usize,
        sample_rate: u32,
        bit_depth: u16,
        max_data_bytes: Option<u64>,
        drop_mode: bool,
        split_size_bytes: Option<u64>,
    ) -> u64 {
        let first_path = if split_size_bytes.is_some() {
            split_file_path(wav_path, 1)
        } else {
            wav_path.to_path_buf()
        };
        let wav_file = open_wav_file(&first_path, channels, sample_rate, bit_depth).unwrap();
        let monitor_sink: Arc<Mutex<Option<ChildStdin>>> = Arc::new(Mutex::new(None));
        let levels = Arc::new(Mutex::new(vec![-60.0f32; channels]));
        let recording_bytes = Arc::new(Mutex::new(0u64));
        let reader = std::io::Cursor::new(pcm_data.to_vec());

        capture_and_analyse(
            reader,
            Some(wav_file),
            Some(wav_path.to_path_buf()),
            monitor_sink,
            channels,
            sample_rate,
            bit_depth,
            levels,
            max_data_bytes,
            drop_mode,
            recording_bytes.clone(),
            0, // min_free_bytes — skip disk check in tests
            split_size_bytes,
        );

        let result = *recording_bytes.lock().unwrap();
        result
    }

    /// Generate fake PCM data (silence) of the given duration.
    fn silence(channels: usize, sample_rate: u32, bit_depth: u16, seconds: f64) -> Vec<u8> {
        let bps = bytes_per_sample(bit_depth);
        let frame_size = channels * bps;
        let num_frames = (sample_rate as f64 * seconds) as usize;
        vec![0u8; num_frames * frame_size]
    }

    #[test]
    fn capture_normal_recording() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("rec.wav");
        let pcm = silence(2, 48000, 32, 1.0); // 1 second stereo 32-bit

        let bytes = run_capture(&pcm, &wav_path, 2, 48000, 32, None, false, None);

        assert_eq!(bytes, pcm.len() as u64);
        // File should be header + data
        let meta = std::fs::metadata(&wav_path).unwrap();
        assert_eq!(meta.len(), HEADER_SIZE + pcm.len() as u64);
        // Header data size should match
        assert_eq!(read_data_size(&wav_path), pcm.len() as u32);
    }

    #[test]
    fn capture_stop_at_limit() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("rec.wav");
        let pcm = silence(2, 48000, 32, 2.0); // 2 seconds
        let frame_size = 2 * 4; // stereo 32-bit
                                // Limit to 1 second worth of data
        let one_sec_bytes = 48000 * frame_size;
        let max_data = HEADER_SIZE + one_sec_bytes as u64;

        let bytes = run_capture(&pcm, &wav_path, 2, 48000, 32, Some(max_data), false, None);

        // Should have stopped at the limit (aligned to frame)
        assert_eq!(bytes, one_sec_bytes as u64);
        let meta = std::fs::metadata(&wav_path).unwrap();
        assert_eq!(meta.len(), HEADER_SIZE + one_sec_bytes as u64);
    }

    #[test]
    fn capture_circular_drop_mode() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("rec.wav");
        // 3 seconds of data into a 1-second buffer
        let pcm = silence(2, 48000, 32, 3.0);
        let frame_size = 2 * 4;
        let one_sec_bytes = 48000 * frame_size;
        let max_data = HEADER_SIZE + one_sec_bytes as u64;

        run_capture(&pcm, &wav_path, 2, 48000, 32, Some(max_data), true, None);

        // After linearization, file should be header + 1 second of data
        let meta = std::fs::metadata(&wav_path).unwrap();
        assert_eq!(meta.len(), HEADER_SIZE + one_sec_bytes as u64);
        assert_eq!(read_data_size(&wav_path), one_sec_bytes as u32);
    }

    #[test]
    fn capture_split_mode() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("rec.wav");
        let pcm = silence(2, 48000, 32, 2.0);
        let frame_size = 2 * 4;
        let one_sec_bytes = (48000 * frame_size) as u64;
        // Split every 1 second
        let split_size = one_sec_bytes;

        run_capture(&pcm, &wav_path, 2, 48000, 32, None, false, Some(split_size));

        // Should have created _001 and _002 files
        let f1 = split_file_path(&wav_path, 1);
        let f2 = split_file_path(&wav_path, 2);
        assert!(f1.exists(), "split file _001 should exist");
        assert!(f2.exists(), "split file _002 should exist");

        // Each file should have header + ~1 second of data
        let size1 = std::fs::metadata(&f1).unwrap().len();
        let size2 = std::fs::metadata(&f2).unwrap().len();
        assert_eq!(size1, HEADER_SIZE + one_sec_bytes);
        // Second file gets remaining data
        assert!(size2 > HEADER_SIZE);
    }

    #[test]
    fn capture_split_drop_mode_evicts_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("rec.wav");
        // 4 seconds of data, split at 1 second, max 2 seconds total
        // Should keep at most 2 files and delete older ones
        let pcm = silence(2, 48000, 32, 4.0);
        let frame_size = 2 * 4;
        let one_sec_bytes = (48000 * frame_size) as u64;
        let split_size = one_sec_bytes;
        let max_data = HEADER_SIZE + 2 * one_sec_bytes; // room for 2 files

        run_capture(
            &pcm,
            &wav_path,
            2,
            48000,
            32,
            Some(max_data),
            true,
            Some(split_size),
        );

        // Oldest files should have been deleted
        let f1 = split_file_path(&wav_path, 1);
        let f2 = split_file_path(&wav_path, 2);
        let f3 = split_file_path(&wav_path, 3);
        let f4 = split_file_path(&wav_path, 4);

        // With max_data = 2 * split, max_split_files = 2.
        // Files 1 and 2 should be evicted, 3 and 4 (or just 4) should remain.
        assert!(!f1.exists(), "oldest file should be deleted");
        assert!(!f2.exists(), "second oldest should be deleted");
        // At least the last file should exist
        assert!(
            f3.exists() || f4.exists(),
            "most recent file(s) should still exist"
        );
    }

    #[test]
    fn capture_empty_input() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("rec.wav");
        let pcm: Vec<u8> = vec![];

        let bytes = run_capture(&pcm, &wav_path, 2, 48000, 32, None, false, None);

        assert_eq!(bytes, 0);
        // File should just be the header
        let meta = std::fs::metadata(&wav_path).unwrap();
        assert_eq!(meta.len(), HEADER_SIZE);
        assert_eq!(read_data_size(&wav_path), 0);
    }
}

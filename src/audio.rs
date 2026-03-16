use std::path::PathBuf;
use std::process::{Command, Child, Stdio};
use std::io::{self, Write, Read, Seek, SeekFrom};
use std::fs::{File, OpenOptions};
use std::time::Instant;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct AudioPlayer {
    process: Option<Child>,
    ffmpeg_process: Option<Child>,
    control_fifo: Option<File>,
    fifo_path: PathBuf,
    analysis_process: Option<Child>,
    start_time: Option<Instant>,
    channel_levels: Arc<Mutex<Vec<f32>>>,
    current_volume: u8,
    recording_process: Option<Child>,
    recording_thread: Option<thread::JoinHandle<()>>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        let fifo_path = PathBuf::from("/tmp/octotrack_mplayer.fifo");
        AudioPlayer {
            process: None,
            ffmpeg_process: None,
            control_fifo: None,
            fifo_path,
            analysis_process: None,
            start_time: None,
            channel_levels: Arc::new(Mutex::new(Vec::new())),
            current_volume: 100,
            recording_process: None,
            recording_thread: None,
        }
    }

    pub fn play(&mut self, track_path: &PathBuf, channel_count: u32, volume: u8, max_volume: u8, eq_bands: &[i8; 10], eq_enabled: bool) -> io::Result<()> {
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
            self.play_multi_file(track_path, channel_count, volume, max_volume, eq_bands, eq_enabled)
        } else {
            self.play_single_file(track_path, channel_count, volume, max_volume, eq_bands, eq_enabled)
        }
    }

    fn setup_control_fifo(&mut self) -> io::Result<()> {
        // Remove existing FIFO if it exists
        let _ = std::fs::remove_file(&self.fifo_path);

        // Create new FIFO using mkfifo command
        let status = Command::new("mkfifo")
            .arg(&self.fifo_path)
            .status()?;

        if !status.success() {
            return Err(io::Error::new(io::ErrorKind::Other, "Failed to create FIFO"));
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
                    p.extension()
                        .map_or(false, |ext| ext.eq_ignore_ascii_case("mp3")
                            || ext.eq_ignore_ascii_case("wav")
                            || ext.eq_ignore_ascii_case("flac"))
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
        cmd.arg("-f").arg("s16le")
           .arg("-ac").arg(channel_count.to_string())
           .arg("-")
           .arg("-loglevel").arg("error");

        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Failed to capture stdout"))?;

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
                            for ch in 0..channel_count {
                                let offset = (i * channel_count + ch) * 2;
                                if offset + 1 < n {
                                    // Read s16le sample
                                    let sample = i16::from_le_bytes([buffer[offset], buffer[offset + 1]]);
                                    // Convert to float (-1.0 to 1.0)
                                    let sample_f = sample as f32 / 32768.0;
                                    channel_buffers[ch].push(sample_f);
                                }
                            }
                        }

                        // Calculate RMS for each channel when we have enough samples
                        let mut updated = false;
                        for ch in 0..channel_count {
                            if channel_buffers[ch].len() >= window_samples {
                                // Calculate RMS
                                let sum_squares: f32 = channel_buffers[ch].iter()
                                    .map(|&s| s * s)
                                    .sum();
                                let rms = (sum_squares / channel_buffers[ch].len() as f32).sqrt();

                                // Convert to dB
                                let db = if rms > 0.0 {
                                    20.0 * rms.log10()
                                } else {
                                    -60.0
                                };

                                current_levels[ch] = db.max(-60.0).min(0.0);
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

    fn play_single_file(&mut self, file_path: &PathBuf, channel_count: u32, volume: u8, max_volume: u8, eq_bands: &[i8; 10], eq_enabled: bool) -> io::Result<()> {
        // Use mplayer's built-in volume control with slave mode for dynamic control
        let mut cmd = Command::new("mplayer");
        cmd.arg("-slave")
            .arg("-quiet")
            .arg("-input")
            .arg(format!("file={}", self.fifo_path.display()))
            .arg("-channels")
            .arg(channel_count.to_string())
            .arg("-softvol")
            .arg("-softvol-max")
            .arg(max_volume.to_string())
            .arg("-volume")
            .arg(volume.to_string());

        // Add EQ filter if enabled
        if eq_enabled {
            cmd.arg("-af")
               .arg(Self::eq_filter_string(eq_bands));
        }

        let process = cmd
            .arg("-ao")
            .arg("alsa:device=hw=0.0")
            .arg(file_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        self.process = Some(process);

        // Give mplayer a moment to open the FIFO
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Open FIFO for writing
        self.control_fifo = Some(OpenOptions::new()
            .write(true)
            .open(&self.fifo_path)?);

        Ok(())
    }

    fn play_multi_file(&mut self, folder_path: &PathBuf, channel_count: u32, volume: u8, max_volume: u8, eq_bands: &[i8; 10], eq_enabled: bool) -> io::Result<()> {
        // Get all audio files in the directory, sorted
        let mut audio_files: Vec<PathBuf> = std::fs::read_dir(folder_path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map_or(false, |ext| ext.eq_ignore_ascii_case("mp3")
                        || ext.eq_ignore_ascii_case("wav")
                        || ext.eq_ignore_ascii_case("flac"))
            })
            .collect();

        audio_files.sort();

        if audio_files.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "No audio files found in directory"));
        }

        // If only one file, play it directly
        if audio_files.len() == 1 {
            return self.play_single_file(&audio_files[0], channel_count, volume, max_volume, eq_bands, eq_enabled);
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

        let mplayer_stdin = ffmpeg_output.stdout.take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Failed to capture ffmpeg stdout"))?;

        let mut mplayer_cmd = Command::new("mplayer");
        mplayer_cmd
            .arg("-slave")
            .arg("-quiet")
            .arg("-input")
            .arg(format!("file={}", self.fifo_path.display()))
            .arg("-channels")
            .arg(channel_count.to_string())
            .arg("-softvol")
            .arg("-softvol-max")
            .arg(max_volume.to_string())
            .arg("-volume")
            .arg(volume.to_string());

        if eq_enabled {
            mplayer_cmd.arg("-af")
                       .arg(Self::eq_filter_string(eq_bands));
        }

        let mplayer_process = mplayer_cmd
            .arg("-ao")
            .arg("alsa:device=hw=0.0")
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
        self.control_fifo = Some(OpenOptions::new()
            .write(true)
            .open(&self.fifo_path)?);

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
            writeln!(fifo, "pausing_keep_force af_cmdline equalizer {}", values.join(":"))?;
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
                writeln!(fifo, "pausing_keep_force af_add {}", Self::eq_filter_string(bands))?;
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
                Ok(Some(_)) => {
                    // Clean up ffmpeg process if mplayer has exited
                    if let Some(mut ffmpeg_process) = self.ffmpeg_process.take() {
                        let _ = ffmpeg_process.kill();
                        let _ = ffmpeg_process.wait();
                    }
                    false
                }
                Ok(None) => true,     // Process is still running
                Err(_) => false,      // Error in checking, assume not running
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

    pub fn start_recording(&mut self, output_path: &PathBuf, input_device: &str, channel_count: u32) -> io::Result<()> {
        self.stop_recording()?;

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Initialise shared levels for the recording channel count
        *self.channel_levels.lock().unwrap() = vec![-60.0; channel_count as usize];

        // arecord outputs raw PCM to stdout — we handle the WAV container ourselves
        // so the same stream can feed both the file writer and the level analyser.
        let mut child = Command::new("arecord")
            .arg("-D").arg(input_device)
            .arg("-c").arg(channel_count.to_string())
            .arg("-r").arg("192000")
            .arg("-f").arg("S32_LE")
            .arg("-t").arg("raw")
            .arg("--buffer-size=65536")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "arecord stdout missing"))?;

        let levels = Arc::clone(&self.channel_levels);
        let out_path = output_path.clone();
        let ch = channel_count as usize;

        let handle = thread::spawn(move || {
            record_and_analyse(stdout, out_path, ch, levels);
        });

        self.recording_process = Some(child);
        self.recording_thread = Some(handle);
        Ok(())
    }

    pub fn stop_recording(&mut self) -> io::Result<()> {
        if let Some(mut process) = self.recording_process.take() {
            let still_running = process.try_wait()?.is_none();
            if still_running {
                // SIGTERM causes arecord to flush its buffer and close stdout cleanly,
                // which lets the writing thread see EOF and finalise the WAV header.
                let pid = process.id();
                let _ = Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = process.wait();
        }
        // Wait for the writing thread to finish — this guarantees the WAV header
        // is fully updated on disk before we return.
        if let Some(handle) = self.recording_thread.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    pub fn is_recording(&mut self) -> bool {
        let exited = match &mut self.recording_process {
            None => return false,
            Some(p) => match p.try_wait() {
                Ok(None) => return true,   // still running
                _ => true,                  // exited or error
            },
        };
        if exited {
            if let Some(mut p) = self.recording_process.take() {
                let _ = p.wait(); // reap zombie
            }
        }
        false
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
        raw_levels.iter()
            .map(|&level| (level + volume_db).max(-60.0).min(0.0))
            .collect()
    }

}

/// Reads raw S32_LE PCM from `reader` (arecord stdout), writes a WAV file to
/// `out_path`, and concurrently updates `levels` with per-channel RMS in dB.
/// Called from a background thread; blocks until EOF (i.e. arecord exits).
fn record_and_analyse<R: Read>(
    mut reader: R,
    out_path: PathBuf,
    channels: usize,
    levels: Arc<Mutex<Vec<f32>>>,
) {
    const SAMPLE_RATE: u32 = 192_000;
    const BITS: u16 = 32;
    const BYTES_PER_SAMPLE: usize = 4; // S32_LE

    let byte_rate = SAMPLE_RATE * channels as u32 * BYTES_PER_SAMPLE as u32;
    let block_align = channels as u16 * BYTES_PER_SAMPLE as u16;
    let frame_size = channels * BYTES_PER_SAMPLE;

    let mut file = match File::create(&out_path) {
        Ok(f) => f,
        Err(_) => return,
    };

    // Write a placeholder WAV header (data size filled in at the end)
    let write_header = |f: &mut File, data_bytes: u32| -> io::Result<()> {
        f.seek(SeekFrom::Start(0))?;
        f.write_all(b"RIFF")?;
        f.write_all(&(data_bytes + 36).to_le_bytes())?;
        f.write_all(b"WAVE")?;
        f.write_all(b"fmt ")?;
        f.write_all(&16u32.to_le_bytes())?;          // PCM subchunk size
        f.write_all(&1u16.to_le_bytes())?;            // AudioFormat = PCM
        f.write_all(&(channels as u16).to_le_bytes())?;
        f.write_all(&SAMPLE_RATE.to_le_bytes())?;
        f.write_all(&byte_rate.to_le_bytes())?;
        f.write_all(&block_align.to_le_bytes())?;
        f.write_all(&BITS.to_le_bytes())?;
        f.write_all(b"data")?;
        f.write_all(&data_bytes.to_le_bytes())?;
        Ok(())
    };

    if write_header(&mut file, 0).is_err() { return; }

    // 100 ms RMS window at 192 kHz
    let window_frames: usize = 19_200;
    let mut ch_bufs: Vec<Vec<f32>> = vec![vec![]; channels];
    let mut cur_levels = vec![-60.0f32; channels];

    // Read buffer: 4096 frames at a time
    let buf_size = frame_size * 4096;
    let mut buf = vec![0u8; buf_size];
    let mut total_bytes: u32 = 0;

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF — arecord exited
            Ok(n) => {
                if file.write_all(&buf[..n]).is_err() { break; }
                total_bytes += n as u32;

                // Level analysis: decode S32_LE frames
                let num_frames = n / frame_size;
                for i in 0..num_frames {
                    for ch in 0..channels {
                        let off = i * frame_size + ch * BYTES_PER_SAMPLE;
                        if off + 3 < n {
                            let s = i32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
                            ch_bufs[ch].push(s as f32 / i32::MAX as f32);
                        }
                    }
                }

                let mut updated = false;
                for ch in 0..channels {
                    if ch_bufs[ch].len() >= window_frames {
                        let rms = (ch_bufs[ch].iter().map(|&s| s * s).sum::<f32>()
                            / ch_bufs[ch].len() as f32)
                            .sqrt();
                        let db = if rms > 0.0 { 20.0 * rms.log10() } else { -60.0 };
                        cur_levels[ch] = db.max(-60.0).min(0.0);
                        ch_bufs[ch].clear();
                        updated = true;
                    }
                }
                if updated {
                    if let Ok(mut g) = levels.lock() { *g = cur_levels.clone(); }
                }
            }
            Err(_) => break,
        }
    }

    // Finalise WAV header with actual data size
    let _ = write_header(&mut file, total_bytes);
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.stop_recording();
        let _ = self.stop();
    }
}

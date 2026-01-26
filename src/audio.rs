use std::path::PathBuf;
use std::process::{Command, Child, Stdio};
use std::io::{self, Write};
use std::fs::{File, OpenOptions};

pub struct AudioPlayer {
    process: Option<Child>,
    ffmpeg_process: Option<Child>,
    control_fifo: Option<File>,
    fifo_path: PathBuf,
}

impl AudioPlayer {
    pub fn new() -> Self {
        let fifo_path = PathBuf::from("/tmp/octotrack_mplayer.fifo");
        AudioPlayer {
            process: None,
            ffmpeg_process: None,
            control_fifo: None,
            fifo_path,
        }
    }

    pub fn play(&mut self, track_path: &PathBuf, channel_count: u32, volume: u8) -> io::Result<()> {
        // Ensure any existing process is stopped before starting a new one
        if let Some(mut process) = self.process.take() {
            process.kill()?;
            process.wait()?;
        }
        if let Some(mut ffmpeg_process) = self.ffmpeg_process.take() {
            ffmpeg_process.kill()?;
            ffmpeg_process.wait()?;
        }
        // Close any open FIFO
        self.control_fifo = None;

        // Setup control FIFO for slave commands
        self.setup_control_fifo()?;

        // Check if track_path is a directory with multiple audio files
        if track_path.is_dir() {
            self.play_multi_file(track_path, channel_count, volume)
        } else {
            self.play_single_file(track_path, channel_count, volume)
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

    fn play_single_file(&mut self, file_path: &PathBuf, channel_count: u32, volume: u8) -> io::Result<()> {
        // Use mplayer's built-in volume control with slave mode for dynamic control
        let process = Command::new("mplayer")
            .arg("-slave")
            .arg("-quiet")
            .arg("-input")
            .arg(format!("file={}", self.fifo_path.display()))
            .arg("-channels")
            .arg(channel_count.to_string())
            .arg("-softvol")
            .arg("-softvol-max")
            .arg("200")
            .arg("-volume")
            .arg(volume.to_string())
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

    fn play_multi_file(&mut self, folder_path: &PathBuf, channel_count: u32, volume: u8) -> io::Result<()> {
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
            return self.play_single_file(&audio_files[0], channel_count, volume);
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

        let mplayer_process = Command::new("mplayer")
            .arg("-slave")
            .arg("-quiet")
            .arg("-input")
            .arg(format!("file={}", self.fifo_path.display()))
            .arg("-channels")
            .arg(channel_count.to_string())
            .arg("-softvol")
            .arg("-softvol-max")
            .arg("200")
            .arg("-volume")
            .arg(volume.to_string())
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

    pub fn stop(&mut self) -> io::Result<()> {
        if let Some(mut process) = self.process.take() {
            process.kill()?;
            process.wait()?;
        }
        if let Some(mut ffmpeg_process) = self.ffmpeg_process.take() {
            ffmpeg_process.kill()?;
            ffmpeg_process.wait()?;
        }
        // Close FIFO and clean up
        self.control_fifo = None;
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
        if let Some(fifo) = &mut self.control_fifo {
            // Send mplayer slave command to set absolute volume
            // Use pausing_keep_force to ensure command works during playback
            writeln!(fifo, "pausing_keep_force volume {} 1", volume)?;
            fifo.flush()?;
        }
        Ok(())
    }

}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        // Clean up processes and FIFO on drop
        let _ = self.stop();
    }
}

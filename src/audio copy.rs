use std::path::PathBuf;
use std::process::{Command, Child, Stdio};
use std::io::{self};

pub struct AudioPlayer {
    process: Option<Child>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        AudioPlayer { process: None}
    }

    pub fn play(&mut self, file_path: &PathBuf) -> io::Result<()> {
        // Ensure any existing process is stopped before starting a new one
        if let Some(mut process) = self.process.take() {
            process.kill()?;
            process.wait()?;
        }

        let channel_count = Command::new("ffprobe")
            .arg("-v")
            .arg("error")
            .arg("-select_streams")
            .arg("a:0")
            .arg("-show_entries")
            .arg("stream=channels")
            .arg("-of")
            .arg("default=noprint_wrappers=1:nokey=1")
            .arg(file_path)
            .output()?
            .stdout;

        let channel_count = String::from_utf8_lossy(&channel_count).trim().parse::<u32>().unwrap_or(0);

        // Start the mplayer process with stdin piped
        let process = Command::new("mplayer")
            .arg("-channels")
            .arg(channel_count.to_string())
            .arg("-ao")
            .arg("alsa:device=hw=0.0")
            .arg("-v")
            .arg(file_path)
            .stdin(Stdio::piped()) // Pipe stdin to send commands
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        self.process = Some(process);
        Ok(())
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if let Some(mut process) = self.process.take() {
            process.kill()?;
            process.wait()?;
        }
        Ok(())
    }


    pub fn is_running(&mut self) -> bool {
        if let Some(process) = &mut self.process {
            match process.try_wait() {
                Ok(Some(_)) => false, // Process has exited
                Ok(None) => true,     // Process is still running
                Err(_) => false,      // Error in checking, assume not running
            }
        } else {
            false
        }
    }

}

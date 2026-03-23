//! First-run setup: interactive password prompts, autostart configuration,
//! and factory reset.
//!
//! Setup runs exactly once — when `config.toml` does not yet exist.
//! After that, use `--set-password` to change passwords / re-run setup, or
//! `--configure-autostart` to reconfigure autostart independently.
//! Use `--reset` to wipe passwords and force first-run on next start.

use crate::config::{toml_path, Config};
use argon2::{
    password_hash::{PasswordHasher, SaltString},
    Argon2,
};
use rand::rngs::OsRng;
use std::io::{self, IsTerminal, Write};

// ---------------------------------------------------------------------------
// Autostart
// ---------------------------------------------------------------------------

const BASHRC_START: &str = "# octotrack autostart (begin)";
const BASHRC_END: &str = "# octotrack autostart (end)";

enum AutostartMethod {
    Systemd,
    Bashrc,
}

enum DisplayType {
    Tft,
    Hdmi,
    Headless,
}

/// Interactive, idempotent autostart configuration.
///
/// Supports two methods:
///   - systemd service: installs `/etc/systemd/system/octotrack.service`
///   - .bashrc autologin: writes a guarded block into `~/.bashrc` and sets
///     up the tty1 autologin drop-in
///
/// Safe to re-run: existing configuration is replaced in-place.
pub fn configure_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy().to_string();

    // Prefer release binary when we're running the debug build.
    let release_exe = {
        let s = exe_str.replace("/target/debug/", "/target/release/");
        if s != exe_str && std::path::Path::new(&s).exists() {
            s
        } else {
            exe_str.clone()
        }
    };

    // Walk up past target/debug or target/release to the project root.
    let workdir = exe
        .parent()
        .and_then(|p| {
            if p.file_name()
                .map(|n| n == "debug" || n == "release")
                .unwrap_or(false)
            {
                p.parent().and_then(|p2| {
                    if p2.file_name().map(|n| n == "target").unwrap_or(false) {
                        p2.parent()
                    } else {
                        Some(p2)
                    }
                })
            } else {
                Some(p)
            }
        })
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_string_lossy()
        .to_string();

    let user = std::env::var("USER").unwrap_or_else(|_| "pi".to_string());

    println!("\n  Autostart configuration");
    println!("  Binary : {}", release_exe);
    println!("  Workdir: {}", workdir);
    println!("  User   : {}", user);
    println!();
    println!("  Method:");
    println!("    1 — systemd service  (recommended, restarts on crash)");
    println!("    2 — .bashrc autologin (simpler, requires tty1 autologin)");
    println!("    3 — skip");

    let method = loop {
        print!("\n  Choice [1/2/3]: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim() {
            "1" => break AutostartMethod::Systemd,
            "2" => break AutostartMethod::Bashrc,
            "3" | "" => return Ok(()),
            _ => eprintln!("  Enter 1, 2, or 3."),
        }
    };

    println!("\n  Display type:");
    println!("    1 — TFT / framebuffer (tty1, 5s boot delay)");
    println!("    2 — HDMI monitor (tty1)");
    println!("    3 — Headless / web only (no TUI)");

    let display = loop {
        print!("\n  Choice [1/2/3]: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim() {
            "1" => break DisplayType::Tft,
            "2" | "" => break DisplayType::Hdmi,
            "3" => break DisplayType::Headless,
            _ => eprintln!("  Enter 1, 2, or 3."),
        }
    };

    match method {
        AutostartMethod::Systemd => {
            remove_bashrc_autostart();
            install_systemd_service(&release_exe, &workdir, &user, display)
        }
        AutostartMethod::Bashrc => {
            remove_systemd_service();
            install_bashrc_autostart(&release_exe, display)
        }
    }
}

fn remove_systemd_service() {
    let path = "/etc/systemd/system/octotrack.service";
    if !std::path::Path::new(path).exists() {
        return;
    }
    let _ = std::process::Command::new("sudo")
        .args(["systemctl", "disable", "--now", "octotrack"])
        .status();
    let _ = std::process::Command::new("sudo")
        .args(["rm", "-f", path])
        .status();
    let _ = std::process::Command::new("sudo")
        .args(["systemctl", "daemon-reload"])
        .status();
    println!("  Removed existing systemd service.");
}

fn remove_bashrc_autostart() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let bashrc_path = format!("{}/.bashrc", home);
    let existing = match std::fs::read_to_string(&bashrc_path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if !existing.contains(BASHRC_START) {
        return;
    }
    let new_content = if let Some(start) = existing.find(BASHRC_START) {
        let end = existing
            .find(BASHRC_END)
            .map(|i| i + BASHRC_END.len())
            .unwrap_or(start);
        let end = if existing[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        format!("{}{}", &existing[..start], &existing[end..])
    } else {
        existing
    };
    let _ = std::fs::write(&bashrc_path, new_content);
    println!("  Removed existing .bashrc autostart block.");
}

fn install_systemd_service(
    exe: &str,
    workdir: &str,
    user: &str,
    display: DisplayType,
) -> Result<(), Box<dyn std::error::Error>> {
    let tty_section = match display {
        DisplayType::Tft => concat!(
            "ExecStartPre=/bin/sleep 5\n",
            "ExecStartPre=+/bin/chvt 1\n",
            "ExecStartPre=/usr/bin/clear\n",
            "StandardInput=tty\n",
            "StandardOutput=tty\n",
            "StandardError=tty\n",
            "TTYPath=/dev/tty1\n",
            "TTYReset=yes\n",
            "TTYVHangup=yes\n",
            "Environment=TERM=linux\n",
        ),
        DisplayType::Hdmi => concat!(
            "ExecStartPre=/bin/sleep 4\n",
            "ExecStartPre=+/bin/chvt 1\n",
            "ExecStartPre=/usr/bin/clear\n",
            "StandardInput=tty\n",
            "StandardOutput=tty\n",
            "StandardError=tty\n",
            "TTYPath=/dev/tty1\n",
            "TTYReset=yes\n",
            "TTYVHangup=yes\n",
            "Environment=TERM=linux\n",
        ),
        DisplayType::Headless => "StandardOutput=journal\nStandardError=journal\n",
    };

    let conflicts = match display {
        DisplayType::Headless => "",
        _ => "Conflicts=getty@tty1.service\nAfter=getty@tty1.service\n",
    };

    let service = format!(
        "[Unit]\n\
         Description=Octotrack Multi-Channel Audio Player\n\
         After=sound.target multi-user.target\n\
         {conflicts}\
         \n\
         [Service]\n\
         Type=simple\n\
         User={user}\n\
         WorkingDirectory={workdir}\n\
         {tty_section}\
         ExecStart={exe}\n\
         AmbientCapabilities=CAP_SYSLOG\n\
         Restart=always\n\
         RestartSec=3\n\
         TimeoutStopSec=10\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    );

    let path = "/etc/systemd/system/octotrack.service";
    if try_sudo_write(path, &service) {
        // Suppress systemd's own late boot console messages (e.g. "Completed
        // socket interaction for boot stage final") that bleed into the TUI.
        patch_systemd_log_level();
        let _ = std::process::Command::new("sudo")
            .args(["systemctl", "daemon-reload"])
            .status();
        let _ = std::process::Command::new("sudo")
            .args(["systemctl", "enable", "octotrack"])
            .status();
        println!("\n  Service installed at {}.", path);
        println!("  To start now: sudo systemctl start octotrack");
    } else {
        println!("\n  Could not write {} (sudo required).", path);
        println!("  Install manually:\n");
        println!("  sudo tee {} << 'EOF'\n{}EOF", path, service);
        println!("  sudo systemctl daemon-reload && sudo systemctl enable --now octotrack");
    }
    Ok(())
}

/// Set systemd's console log level to warning so late boot messages don't
/// bleed into the TUI. Uses `sed` to update the existing `LogLevel=` line
/// if present, otherwise appends it under `[Manager]`.
fn patch_systemd_log_level() {
    let conf_path = "/etc/systemd/system.conf";
    let Ok(content) = std::fs::read_to_string(conf_path) else {
        return;
    };

    let already_set = content.lines().any(|l| l.trim() == "LogLevel=warning");

    if already_set {
        return;
    }

    // Replace any existing LogLevel= line (commented or not), or append.
    let new_content = if content
        .lines()
        .any(|l| l.trim_start_matches('#').trim().starts_with("LogLevel="))
    {
        content
            .lines()
            .map(|l| {
                if l.trim_start_matches('#').trim().starts_with("LogLevel=") {
                    "LogLevel=warning".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        format!("{}\nLogLevel=warning\n", content.trim_end())
    };

    if try_sudo_write(conf_path, &new_content) {
        println!("  Set systemd LogLevel=warning in {}.", conf_path);
    }
}

fn install_bashrc_autostart(
    exe: &str,
    display: DisplayType,
) -> Result<(), Box<dyn std::error::Error>> {
    let run_cmd = match display {
        DisplayType::Tft | DisplayType::Hdmi => exe.to_string(),
        DisplayType::Headless => format!("{} --headless", exe),
    };

    let block = format!(
        "{start}\nif [ \"$(tty)\" = \"/dev/tty1\" ]; then\n    sleep 4\n    clear\n    {cmd}\nfi\n{end}\n",
        start = BASHRC_START,
        cmd = run_cmd,
        end = BASHRC_END,
    );

    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let bashrc_path = format!("{}/.bashrc", home);

    let existing = std::fs::read_to_string(&bashrc_path).unwrap_or_default();
    let new_content = if let Some(start) = existing.find(BASHRC_START) {
        let end = existing
            .find(BASHRC_END)
            .map(|i| i + BASHRC_END.len())
            .unwrap_or(start);
        let end = if existing[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        format!("{}{}{}", &existing[..start], block, &existing[end..])
    } else {
        format!("{}\n{}", existing.trim_end(), block)
    };

    std::fs::write(&bashrc_path, new_content)?;
    println!("\n  Updated {}.", bashrc_path);

    // Set up the tty1 autologin drop-in.
    let autologin_dir = "/etc/systemd/system/getty@tty1.service.d";
    let autologin_conf = format!("{}/autologin.conf", autologin_dir);
    let user = std::env::var("USER").unwrap_or_else(|_| "pi".to_string());
    let autologin_content = format!(
        "[Service]\nExecStart=\nExecStart=-/sbin/agetty --autologin {} --noclear %I $TERM\n",
        user
    );

    let already_set = std::fs::read_to_string(&autologin_conf)
        .map(|s| s.contains(&format!("--autologin {}", user)))
        .unwrap_or(false);

    if already_set {
        println!("  tty1 autologin already configured.");
    } else if try_sudo_mkdir(autologin_dir) && try_sudo_write(&autologin_conf, &autologin_content) {
        let _ = std::process::Command::new("sudo")
            .args(["systemctl", "daemon-reload"])
            .status();
        println!("  tty1 autologin configured.");
    } else {
        println!("  Could not configure autologin (sudo required).");
        println!("  Set it up manually:\n");
        println!("  sudo mkdir -p {}", autologin_dir);
        println!(
            "  sudo tee {} << 'EOF'\n{}EOF",
            autologin_conf, autologin_content
        );
        println!("  sudo systemctl daemon-reload");
    }

    Ok(())
}

fn try_sudo_write(path: &str, content: &str) -> bool {
    use std::io::Write as _;
    let Ok(mut child) = std::process::Command::new("sudo")
        .args(["tee", path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(content.as_bytes());
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

fn try_sudo_mkdir(path: &str) -> bool {
    std::process::Command::new("sudo")
        .args(["mkdir", "-p", path])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Returns `true` when first-run setup must run.
/// This is only the case when `config.toml` does not exist yet.
pub fn needs_setup(_config: &Config) -> bool {
    !toml_path().exists()
}

/// Run the interactive first-run setup prompt.
///
/// Saves the updated config to disk and returns it on success.
/// Exits the process with an error message if there is no TTY (e.g. systemd
/// before setup has been completed).
pub fn run_setup(mut config: Config) -> Result<Config, Box<dyn std::error::Error>> {
    if !io::stdin().is_terminal() {
        eprintln!(
            "error: octotrack requires first-run setup but no terminal is available.\n\
             Run `octotrack --set-password` interactively to complete setup, then restart."
        );
        std::process::exit(1);
    }

    println!("octotrack — first run setup\n");

    // Web UI
    if prompt_yes_no("  Enable web interface?", true)? {
        config.web.enabled = true;
        println!("  Web UI password");
        let web_pass = prompt_confirmed("  Enter password: ", "  Confirm:        ", 1)?;
        config.web.password_hash = hash_password(&web_pass)?;
    } else {
        config.web.enabled = false;
        config.web.password_hash = String::new();
    }

    // Access point
    println!();
    if prompt_yes_no("  Enable WiFi access point?", true)? {
        config.network.ap.enabled = true;
        println!("  Access point password (min 8 characters)");
        let ap_pass = prompt_confirmed("  Enter password: ", "  Confirm:        ", 8)?;
        config.network.ap.password = ap_pass;
    } else {
        config.network.ap.enabled = false;
        config.network.ap.password = String::new();
    }

    config.save()?;

    // Autostart
    println!();
    configure_autostart()?;

    println!("\n  Setup complete. Starting octotrack...");
    if config.network.ap.enabled {
        println!("  AP network : {}", config.network.ap.ssid);
    }
    if config.web.enabled {
        println!(
            "  Web UI     : http://{}.local:{}",
            config.web.hostname, config.web.port
        );
    }

    Ok(config)
}

/// Clear both password fields and save, forcing `--set-password` to be run
/// before the web UI or AP will work again.
pub fn factory_reset(mut config: Config) -> Result<(), Box<dyn std::error::Error>> {
    config.web.enabled = true;
    config.web.password_hash = String::new();
    config.network.ap.enabled = true;
    config.network.ap.password = String::new();
    config.save()?;
    println!("Passwords cleared. Run `octotrack --set-password` to reconfigure.");
    Ok(())
}

/// Hash a password with Argon2id, returning a PHC string.
pub fn hash_password(password: &str) -> Result<String, Box<dyn std::error::Error>> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| format!("Failed to hash password: {e}"))?;
    Ok(hash.to_string())
}

/// Print a yes/no prompt and return the user's choice.
/// `default` is used when the user presses Enter without typing anything.
fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        print!("{prompt} {hint}: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim().to_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => eprintln!("  Please enter y or n."),
        }
    }
}

/// Prompt for a password with a confirmation loop.
/// `min_len` — minimum accepted length (1 = any non-empty password).
fn prompt_confirmed(
    prompt: &str,
    confirm_prompt: &str,
    min_len: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    loop {
        let pass = rpassword::prompt_password(prompt)?;
        if pass.len() < min_len {
            eprintln!("  Password must be at least {min_len} character(s). Try again.");
            continue;
        }
        let confirm = rpassword::prompt_password(confirm_prompt)?;
        if pass == confirm {
            return Ok(pass);
        }
        eprintln!("  Passwords do not match. Try again.");
    }
}

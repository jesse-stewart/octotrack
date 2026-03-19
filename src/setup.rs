//! First-run setup: interactive password prompts and factory reset.
//!
//! Setup runs exactly once — when `config.toml` does not yet exist.
//! After that, use `--set-password` to change passwords or `--reset` to
//! wipe passwords and re-run setup on next start.

use crate::config::{toml_path, Config};
use argon2::{
    password_hash::{PasswordHasher, SaltString},
    Argon2,
};
use rand::rngs::OsRng;
use std::io::{self, IsTerminal, Write};

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

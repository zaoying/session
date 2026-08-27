use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Result};

pub fn home_dir() -> String {
    match std::env::consts::OS {
        "linux" | "macos" | "android" => std::env::var("HOME"),
        "windows" => std::env::var("USERPROFILE"),
        other => panic!("unsupported os: {}", other),
    }
    .expect("failed to get user home dir")
}

fn session_path() -> String {
    Path::new(&home_dir())
        .join(".session")
        .to_str()
        .unwrap()
        .to_string()
}

fn known_hosts_path() -> String {
    Path::new(&home_dir())
        .join(".ssh")
        .join("known_hosts")
        .to_str()
        .unwrap()
        .to_string()
}

/// Load saved sessions from ~/.session, deduplicated, order preserved.
/// No output to stdout.
pub fn load_stored_sessions() -> Result<Vec<String>> {
    let path = session_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for line in content.lines() {
        let s = line.trim().to_string();
        if s.is_empty() {
            continue;
        }
        if !seen.contains(&s) {
            seen.insert(s.clone());
            out.push(s);
        }
    }
    Ok(out)
}

/// List unique hosts from ~/.ssh/known_hosts.
/// No output to stdout.
pub fn list_known_hosts() -> Result<Vec<String>> {
    let path = known_hosts_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let host = line.split(' ').next().unwrap_or("").to_string();
        if host.is_empty() {
            continue;
        }
        if !seen.contains(&host) {
            seen.insert(host.clone());
            out.push(host);
        }
    }
    Ok(out)
}

/// Append a session to ~/.session if not already present.
pub fn save_session(session: &str) -> Result<()> {
    let session = session.trim();
    if session.is_empty() {
        return Ok(());
    }
    let sessions = load_stored_sessions()?;
    if sessions.iter().any(|s| s == session) {
        return Ok(());
    }
    let path = session_path();
    let mut file = OpenOptions::new()
        .write(true)
        .append(true)
        .create(true)
        .open(&path)?;
    writeln!(file, "{}", session)?;
    Ok(())
}

/// Remove the session at `index` (0-based) from ~/.session.
/// Returns the removed session string.
pub fn remove_session(index: usize) -> Result<String> {
    let mut sessions = load_stored_sessions()?;
    if index >= sessions.len() {
        bail!("index out of bounds: {}", index);
    }
    let removed = sessions.remove(index);
    write_sessions(&sessions)?;
    Ok(removed)
}

fn write_sessions(sessions: &[String]) -> Result<()> {
    let path = session_path();
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(&path)?;
    for s in sessions {
        writeln!(file, "{}", s)?;
    }
    Ok(())
}

/// Remove the known_hosts entry at `index` (0-based).
/// Returns the removed host.
pub fn remove_known_host(index: usize) -> Result<String> {
    let mut hosts = list_known_hosts()?;
    if index >= hosts.len() {
        bail!("index out of bounds: {}", index);
    }
    let removed = hosts.remove(index);

    // Rewrite the full known_hosts file, preserving only non-matching lines.
    // Since we only have unique host list, we filter out lines starting with the removed host.
    let path = known_hosts_path();
    let content = fs::read_to_string(&path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(&path)?;
    for line in content.lines() {
        let host = line.split(' ').next().unwrap_or("");
        if host != removed {
            writeln!(file, "{}", line)?;
        }
    }
    Ok(removed)
}

/// Spawn an SSH session and wait for it to finish.
pub fn ssh_login(target: &str) -> io::Result<std::process::ExitStatus> {
    let target = target.trim();
    if target.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty target"));
    }
    let cmd = target.trim_start_matches("ssh ").trim_start_matches("ssh");
    Command::new("ssh").arg(cmd).status()
}

/// Run ssh-copy-id to install public key on the remote host.
pub fn ssh_copy_id(target: &str) -> io::Result<std::process::ExitStatus> {
    let target = target.trim();
    if target.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty target"));
    }
    let cmd = target.trim_start_matches("ssh ").trim_start_matches("ssh");
    Command::new("ssh-copy-id").arg(cmd).status()
}

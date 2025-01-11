use std::{collections::{hash_map::RandomState, HashSet}, fs::{self, OpenOptions}, io::{self, Write}, ops::AddAssign, path};
use std::process::Command;
use anyhow::bail;
use anyhow::Result;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    static ref NUMBER: Regex = Regex::new(r"^-?\d+$").unwrap();
}

/// Locates and returns the user's home directory path
///
/// # Returns
///
/// - `String`: The path to the user's home directory
///
/// # Panics
///
/// - If the operating system is not supported
/// - If the home directory cannot be determined
pub fn locate_home_dir() -> String {
    let home = match std::env::consts::OS {
        "linux" => std::env::var("HOME"),
        "macos" => std::env::var("HOME"),
        "android" => std::env::var("HOME"),
        "windows" => std::env::var("USERPROFILE"),
        other => panic!("unsupported os: {:}", other)
    };
    return home.expect("failed to get user home dir");
}

/// Lists all known SSH hosts from the user's known_hosts file
///
/// # Behavior
///
/// - Reads the known_hosts file from ~/.ssh/known_hosts
/// - Parses and deduplicates host entries
/// - Prints a numbered list of unique hosts
pub fn list_known_hosts(offset: usize) -> Result<Vec<String>> {
    let home = locate_home_dir();
    let binding = path::Path::new(home.as_str()).join(".ssh").join("known_hosts");
    let path = binding.to_str().unwrap();
    let known_hosts = fs::read_to_string(path).expect("failed to read file");
    let mut ip_map: HashSet<String, RandomState> = HashSet::new();
    let mut ip_array: Vec<String> = Vec::new();
    for host in known_hosts.lines().into_iter() {
        if host.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = host.split(' ').collect();
        let ip = cols[0];
        if !ip_map.contains(ip) {
            ip_map.insert(ip.to_string());
            ip_array.push(ip.to_string());
        }
    }

    let mut index: usize = 0;
    for ip in ip_array.iter() {
        println!("{}: {}", index + offset + 1, ip);
        index.add_assign(1);
    }
    return Ok(ip_array);
}

pub fn load_stored_session() -> Result<Vec<String>> {
    let home = locate_home_dir();
    let binding = path::Path::new(home.as_str()).join(".session");
    let path = binding.to_str().unwrap();
    let sessions = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => String::new()
    };
    let mut ip_map: HashSet<String, RandomState> = HashSet::new();
    let mut ip_array: Vec<String> = Vec::new();

    for session in sessions.lines().into_iter() {
        if session.trim().is_empty() {
            continue;
        }
        if !ip_map.contains(session) {
            ip_map.insert(session.to_string());
            ip_array.push(session.to_string());
        }
    }

    let mut index: usize = 0;
    for ip in ip_array.iter() {
        println!("{}: {}", index + 1, ip);
        index.add_assign(1);
    }
    return Ok(ip_array);
}

/// Prompts the user to select a host from the listed known hosts
///
/// # Behavior
///
/// - Displays a prompt asking for a host index
/// - Reads user input from stdin
/// - Validates input as a positive integer
/// - Prints the selected number or an error message for invalid input
pub fn read_prompt() -> Result<String> {
    println!("List stored sessions from '~/.session': ");
    let sessions = load_stored_session().expect("failed to load sessions");
    println!("-----------------------------------------");
    println!("* Enter number to open stored session;");
    println!("* Enter 'username@host' to open new session;");
    println!("* Enter nothing to list hosts from '~/.ssh/known_host';");
    
    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("Failed to read line");

    let mut known_hosts: Vec<String> = Vec::new();
    if input.trim().is_empty() {
        known_hosts = list_known_hosts(sessions.len()).expect("failed to list know_host");
        io::stdin().read_line(&mut input).expect("Failed to read line");
    }
    let input = input.trim();
    
    if NUMBER.is_match(input) {
        let index = match input.parse::<usize>(){
            Ok(numb) => numb,
            Err(e) => bail!("failed to parse number: {}", e)
        };
        if index <= 0 {
            bail!("index outbound: {}", index)
        }
        if 0 < index && index <= sessions.len() {
            return match sessions.get(index - 1) {
                Some(ip) => Ok(ip.to_string()),
                None => bail!("failed to read from known hosts")
            }
        }
        if !known_hosts.is_empty() {
            let index = index - sessions.len();
            if 0 < index && index <= known_hosts.len() {
                return match known_hosts.get(index - 1) {
                    Some(ip) => Ok(ip.to_string()),
                    None => bail!("failed to read from known hosts")
                }
            }
        }
        bail!("index outbound: {}", index)
    }
    
    if let Err(_) = sessions.binary_search(&input.to_string()) {
        save_session(input.to_string());
    }
    return Ok(input.to_string());
}

pub fn save_session(session: String) {
    if session.is_empty() {
        return;
    }
    let home = locate_home_dir();
    let binding = path::Path::new(home.as_str()).join(".session");
    let path = binding.to_str().unwrap();

    let mut file = OpenOptions::new()
    .write(true)
    .append(true)
    .create(true)  // 如果文件不存在则创建
    .open(path).expect("failed to open .session");
    
    let content = session + "\n";
    file.write_all(content.as_bytes()).expect("failed to save session");
}

pub fn ssh_login(session: String) -> io::Result<usize> {
    if session.is_empty() {
        return Ok(0);
    }
    let mut cmd = session.clone();
    if !session.contains("@") {
        println!("Enter username to login {} :", session);
        let mut username = String::new();
        io::stdin().read_line(&mut username).expect("Failed to get username");
        let username = username.trim();
        cmd = format!("{}@{}", username, session);
        save_session(cmd.clone());
    }
    let cmd = cmd.replace(r"^ssh", "");
    let child = Command::new("ssh")
        .arg(cmd)
        .spawn() 
        .expect("Failed to spawn command");

    let output = child.wait_with_output().expect("Failed to wait on child");

    if output.status.success() {
        io::stdout().write(&output.stdout)
    } else {
        io::stderr().write(&output.stderr)
    }
}

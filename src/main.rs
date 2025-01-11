use session::ssh_login;

fn main() {
    match session::read_prompt() {
        Ok(session) => ssh_login(session).expect("ssh login failed"),
        Err(e) => {
            eprintln!("failed to read prompt: {}", e);
            0
        }
    };
}

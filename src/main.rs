use session::ssh_login;

fn main() {
    let session: String = session::read_prompt().expect("failed to read prompt");

    ssh_login(session).expect("ssh login failed");
}

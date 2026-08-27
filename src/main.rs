use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// Primary mode — type an address, pick from list, Enter to connect.
/// Like vi's insert mode: you're always ready to type.
struct Connect {
    input: String,
    /// Filtered target list (saved sessions + maybe known hosts, per filter rules).
    filtered: Vec<String>,
    /// Index into `filtered` selected via Tab/up/down.
    selected: Option<usize>,
}

/// Secondary mode — manage saved sessions (delete, renew key, forward key).
/// Entered via `/edit` in connect mode. Like vi's normal mode.
struct Edit {
    list_state: ListState,
    /// Per-session checked state (aligned with app.sessions indices).
    checked: Vec<bool>,
    /// Pending operation to apply to checked items when S is pressed.
    pending_op: Option<ConfirmAction>,
}

enum Mode {
    Connect(Connect),
    Edit(Edit),
}

#[derive(Debug, Clone, Copy)]
enum ConfirmAction {
    Delete,
    Renew,
    Forward,
}

enum PendingAction {
    SshConnect(String),
    RenewKey(String),
    SendPubkey(String),
}

struct App {
    sessions: Vec<String>,
    known_hosts: Vec<String>,
    mode: Mode,
    status: String,
    should_quit: bool,
    pending: Option<PendingAction>,
}

impl App {
    fn new() -> Result<Self> {
        let sessions = session::load_stored_sessions().unwrap_or_default();
        let known_hosts = session::list_known_hosts().unwrap_or_default();
        let filtered = Self::filter_targets(&sessions, &known_hosts, "");
        let selected = if filtered.is_empty() { None } else { Some(0) };
        Ok(Self {
            sessions,
            known_hosts,
            mode: Mode::Connect(Connect {
                input: String::new(),
                filtered,
                selected,
            }),
            status: default_connect_status(),
            should_quit: false,
            pending: None,
        })
    }

    // --- mode switching ----------------------------------------------------

    fn enter_edit_mode(&mut self) {
        let mut list_state = ListState::default();
        if !self.sessions.is_empty() {
            list_state.select(Some(0));
        }
        self.mode = Mode::Edit(Edit {
            list_state,
            checked: vec![false; self.sessions.len()],
            pending_op: None,
        });
        self.status = default_edit_status();
    }

    fn enter_connect_mode(&mut self) {
        let sessions = self.sessions.clone();
        let known_hosts = self.known_hosts.clone();
        let filtered = Self::filter_targets(&sessions, &known_hosts, "");
        let selected = if filtered.is_empty() { None } else { Some(0) };
        self.mode = Mode::Connect(Connect {
            input: String::new(),
            filtered,
            selected,
        });
        self.status = default_connect_status();
    }

    // --- navigation --------------------------------------------------------

    fn move_selection(&mut self, delta: i32) {
        match &mut self.mode {
            Mode::Connect(Connect {
                input,
                filtered,
                selected,
            }) => {
                if filtered.is_empty() {
                    *selected = None;
                    return;
                }
                let cur = selected.unwrap_or(0);
                let next = (cur as i32 + delta).rem_euclid(filtered.len() as i32) as usize;
                let chosen = filtered[next].clone();
                *selected = Some(next);

                // Update input to reflect the chosen target.
                if chosen.contains('@') {
                    *input = chosen;
                } else if let Some(pos) = input.rfind('@') {
                    input.truncate(pos + 1); // keep "user@"
                    input.push_str(&chosen);
                } else if !input.is_empty() {
                    input.push('@');
                    input.push_str(&chosen);
                } else {
                    *input = chosen;
                }
            }
            Mode::Edit(edit) => {
                let n = self.sessions.len();
                if n == 0 {
                    return;
                }
                let cur = edit.list_state.selected().unwrap_or(0);
                let next = (cur as i32 + delta).rem_euclid(n as i32) as usize;
                edit.list_state.select(Some(next));
            }
        }
    }

    fn reload(&mut self) {
        self.sessions = session::load_stored_sessions().unwrap_or_default();
        self.known_hosts = session::list_known_hosts().unwrap_or_default();
        let sessions = self.sessions.clone();
        let known_hosts = self.known_hosts.clone();
        match &mut self.mode {
            Mode::Connect(Connect {
                input,
                filtered,
                selected,
            }) => {
                *filtered = Self::filter_targets(&sessions, &known_hosts, input);
                let n = filtered.len();
                if n == 0 {
                    *selected = None;
                } else if selected.unwrap_or(0) >= n {
                    *selected = Some(n - 1);
                }
            }
            Mode::Edit(edit) => {
                let n = self.sessions.len();
                let cur = edit.list_state.selected().unwrap_or(0);
                if n == 0 {
                    edit.list_state.select(None);
                } else if cur >= n {
                    edit.list_state.select(Some(n - 1));
                }
                edit.checked.resize(n, false);
            }
        }
    }

    // --- connect mode actions ----------------------------------------------

    fn connect_push_char(&mut self, c: char) {
        let Mode::Connect(conn) = &mut self.mode else {
            return;
        };
        conn.input.push(c);
        // Filter is NOT auto-applied — press Tab to filter.
    }

    fn connect_backspace(&mut self) {
        let Mode::Connect(conn) = &mut self.mode else {
            return;
        };
        conn.input.pop();
    }

    fn connect_trigger_filter(&mut self) {
        let sessions = self.sessions.clone();
        let known_hosts = self.known_hosts.clone();
        let Mode::Connect(conn) = &mut self.mode else {
            return;
        };
        conn.filtered = Self::filter_targets(&sessions, &known_hosts, &conn.input);
        conn.selected = if conn.filtered.is_empty() {
            None
        } else {
            Some(0)
        };
        self.status = format!("filtered: {} match(es)", conn.filtered.len());
    }

    fn connect_confirm(&mut self) {
        let Mode::Connect(Connect {
            input,
            filtered,
            selected,
        }) = &self.mode
        else {
            return;
        };

        // Input is the single source of truth.
        // Only fall back to the selected list item if input is empty.
        let input_text = if !input.trim().is_empty() {
            input.trim().to_string()
        } else if let Some(idx) = selected {
            filtered[*idx].clone()
        } else {
            return;
        };
        if input_text.is_empty() {
            return;
        }

        // Built-in commands start with '/'
        if let Some(cmd) = input_text.strip_prefix('/') {
            match cmd.to_ascii_lowercase().as_str() {
                "edit" => {
                    self.enter_edit_mode();
                    return;
                }
                "exit" | "quit" => {
                    self.should_quit = true;
                    return;
                }
                _ => {
                    self.status = format!("unknown command: /{}", cmd);
                    return;
                }
            }
        }

        // Normal target — save session and connect via ssh
        let _ = session::save_session(&input_text);
        self.pending = Some(PendingAction::SshConnect(input_text));
        self.should_quit = true;
    }

    // --- edit mode actions -------------------------------------------------

    fn edit_toggle_check(&mut self) {
        let Mode::Edit(edit) = &mut self.mode else {
            return;
        };
        let Some(idx) = edit.list_state.selected() else {
            return;
        };
        if let Some(checked) = edit.checked.get_mut(idx) {
            *checked = !*checked;
        }
    }

    fn edit_mark_op(&mut self, op: ConfirmAction) {
        let Mode::Edit(edit) = &mut self.mode else {
            return;
        };
        if !edit.checked.iter().any(|&c| c) {
            self.status = "no items selected — press Space to check".to_string();
            return;
        }
        edit.pending_op = Some(op);
        let name = match op {
            ConfirmAction::Delete => "Delete",
            ConfirmAction::Renew => "Renew key",
            ConfirmAction::Forward => "Forward key",
        };
        let count = edit.checked.iter().filter(|&&c| c).count();
        self.status = format!(
            "{} marked for {} — press [S] to confirm, Esc cancels",
            count, name
        );
    }

    fn edit_apply_pending(&mut self) {
        let Mode::Edit(edit) = &mut self.mode else {
            return;
        };
        let Some(op) = edit.pending_op.take() else {
            self.status = "no pending operation".to_string();
            return;
        };

        let mut indices: Vec<usize> = edit
            .checked
            .iter()
            .enumerate()
            .filter(|(_, &c)| c)
            .map(|(i, _)| i)
            .collect();

        if indices.is_empty() {
            self.status = "no items selected".to_string();
            return;
        }

        match op {
            ConfirmAction::Delete => {
                // Delete highest index first so lower indices stay valid.
                indices.sort_by(|a, b| b.cmp(a));
                let mut count = 0usize;
                for idx in &indices {
                    if session::remove_session(*idx).is_ok() {
                        count += 1;
                    }
                }
                self.status = format!("deleted {} session(s)", count);
                self.reload();
            }
            ConfirmAction::Renew => {
                let sessions = self.sessions.clone();
                let mut removed_any = false;
                for idx in &indices {
                    let Some(s) = sessions.get(*idx) else {
                        continue;
                    };
                    let host = s.rsplit('@').next().unwrap_or(s).to_string();
                    if let Some(kh_idx) = self.known_hosts.iter().position(|h| h == &host) {
                        let _ = session::remove_known_host(kh_idx);
                        removed_any = true;
                    }
                }
                self.reload();
                if let Some(&first_idx) = indices.first() {
                    if let Some(target) = sessions.get(first_idx).cloned() {
                        self.status = format!(
                            "renewed {} host key(s) — connecting to {}",
                            indices.len(),
                            target
                        );
                        self.pending = Some(PendingAction::RenewKey(target));
                        self.should_quit = true;
                        return;
                    }
                }
                if removed_any {
                    self.status = "renewed host keys".to_string();
                } else {
                    self.status = "no known_hosts entries found".to_string();
                }
            }
            ConfirmAction::Forward => {
                // ssh-copy-id to the first selected session only
                // (can't run multiple interactively).
                if let Some(&first_idx) = indices.first() {
                    if let Some(target) = self.sessions.get(first_idx).cloned() {
                        self.status = format!("forwarding key to {} ...", target);
                        self.pending = Some(PendingAction::SendPubkey(target));
                        self.should_quit = true;
                    }
                }
            }
        }
    }

    // --- filtering ---------------------------------------------------------

    /// Filter rules (keyword is trimmed before filtering):
    /// 1. Empty keyword → saved sessions only (fallback to known hosts if none).
    /// 2. Non-empty keyword → filter saved sessions with full keyword.
    ///    2.1 If no saved sessions match → show known hosts; if keyword has @,
    ///        filter known hosts by chars after @.
    /// Account filter does NOT exclude known hosts (they have no account).
    fn filter_targets(sessions: &[String], known_hosts: &[String], keyword: &str) -> Vec<String> {
        let keyword = keyword.trim();
        let keyword_lc = keyword.to_lowercase();

        if keyword.is_empty() {
            if !sessions.is_empty() {
                return sessions.to_vec();
            }
            // Fallback: no saved sessions → show known hosts.
            return known_hosts.to_vec();
        }

        // Step 1: filter saved sessions with full keyword
        let saved_matches: Vec<String> = sessions
            .iter()
            .filter(|s| s.to_lowercase().contains(&keyword_lc))
            .cloned()
            .collect();

        if !saved_matches.is_empty() {
            return saved_matches;
        }

        // No saved match → show known hosts (filtered by host part if @ present)
        let has_at = keyword.contains('@');
        let host_filter = match keyword.rfind('@') {
            Some(pos) => keyword[pos + 1..].to_lowercase(),
            None => String::new(),
        };

        let mut results = Vec::new();
        for h in known_hosts {
            if !has_at || host_filter.is_empty() || h.to_lowercase().contains(&host_filter) {
                results.push(h.clone());
            }
        }
        results
    }
}

// --- status helpers -------------------------------------------------------

fn default_connect_status() -> String {
    "type user@host, Tab filter, ↑/↓ pick, Enter connect, /edit manage sessions".to_string()
}

fn default_edit_status() -> String {
    "[Space] check  [D]elete  [R]enew  [F]orward  [S]ave  [Esc] exit".to_string()
}

// --- main -----------------------------------------------------------------

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new()?;
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }

    // Execute pending action after TUI is torn down.
    if let Some(action) = app.pending {
        match action {
            PendingAction::SshConnect(target) => {
                println!("Connecting to {} ...", target);
                let _ = session::ssh_login(&target);
            }
            PendingAction::RenewKey(target) => {
                println!(
                    "Reconnecting to {} (host key removed, SSH will re-add it) ...",
                    target
                );
                let _ = session::ssh_login(&target);
            }
            PendingAction::SendPubkey(target) => {
                println!("Sending public key to {} ...", target);
                match session::ssh_copy_id(&target) {
                    Ok(code) => println!("ssh-copy-id exited with code {}", code),
                    Err(e) => eprintln!("ssh-copy-id failed: {}", e),
                }
                println!("\nPress Enter to exit...");
                let mut buf = String::new();
                let _ = io::stdin().read_line(&mut buf);
            }
        }
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match &app.mode {
                // === Connect mode (main / "insert" mode) ===
                Mode::Connect { .. } => match key.code {
                    KeyCode::Enter => {
                        app.connect_confirm();
                    }
                    KeyCode::Backspace => {
                        app.connect_backspace();
                    }
                    KeyCode::Down => {
                        app.move_selection(1);
                    }
                    KeyCode::Up => {
                        app.move_selection(-1);
                    }
                    KeyCode::Tab => {
                        app.connect_trigger_filter();
                    }
                    KeyCode::BackTab => {
                        app.move_selection(-1);
                    }
                    KeyCode::Esc => {
                        app.status = default_connect_status();
                    }
                    KeyCode::Char(c) => {
                        app.connect_push_char(c);
                    }
                    _ => {}
                },

                // === Edit mode (like vi normal mode) ===
                Mode::Edit { .. } => match key.code {
                    KeyCode::Char('q') => {
                        app.enter_connect_mode();
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        app.move_selection(1);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        app.move_selection(-1);
                    }
                    KeyCode::Char(' ') => {
                        app.edit_toggle_check();
                    }
                    KeyCode::Char('d' | 'D') => {
                        app.edit_mark_op(ConfirmAction::Delete);
                    }
                    KeyCode::Char('r' | 'R') => {
                        app.edit_mark_op(ConfirmAction::Renew);
                    }
                    KeyCode::Char('f' | 'F') => {
                        app.edit_mark_op(ConfirmAction::Forward);
                    }
                    KeyCode::Char('s' | 'S') => {
                        app.edit_apply_pending();
                    }
                    KeyCode::Esc => {
                        // Esc clears pending op first, then exits on second press
                        let Mode::Edit(edit) = &mut app.mode else {
                            return Ok(());
                        };
                        if edit.pending_op.is_some() {
                            edit.pending_op = None;
                            app.status = default_edit_status();
                        } else {
                            app.enter_connect_mode();
                        }
                    }
                    KeyCode::Char('i' | 'a' | '/') => {
                        app.enter_connect_mode();
                    }
                    _ => {}
                },
            }

            if app.should_quit {
                return Ok(());
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let full = f.size();
    let width = 70.min(full.width.saturating_sub(4));
    let x = (full.width - width) / 2;

    match &app.mode {
        Mode::Connect { .. } => ui_connect(f, app, x, full, width),
        Mode::Edit { .. } => ui_edit(f, app, x, full, width),
    }
}

// ---------------------------------------------------------------------------
// Connect mode UI — input + filtered targets list
// ---------------------------------------------------------------------------

fn ui_connect(
    f: &mut Frame,
    app: &mut App,
    x: u16,
    full: ratatui::layout::Rect,
    width: u16,
) {
    let Mode::Connect(conn) = &app.mode else {
        return;
    };

    let mut state = ListState::default();
    if let Some(i) = conn.selected {
        state.select(Some(i));
    }

    let n = conn.filtered.len();
    let list_h = n.max(3).min(12) as u16 + 2; // +2 for borders
    let total_h = 1 + 3 + list_h + 1; // title + input + list + help
    let total_h = total_h.min(full.height.saturating_sub(2));
    let y = (full.height - total_h) / 2;
    let area = ratatui::layout::Rect::new(x, y, width, total_h);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Length(3), // input box
            Constraint::Min(3),    // filtered targets
            Constraint::Length(1), // help
        ])
        .split(area);

    // Title
    let title = Paragraph::new(vec![Line::from(vec![
        Span::styled(
            " session ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::from(format!(" {} saved", app.sessions.len())),
    ])]);
    f.render_widget(title, chunks[0]);

    // Input box
    let cursor_suffix = if conn.input.is_empty() {
        Span::styled(" ", Style::default().bg(Color::Cyan))
    } else {
        Span::from("")
    };
    let input_line = if let Some(pos) = conn.input.rfind('@') {
        let user = &conn.input[..pos + 1];
        let host = &conn.input[pos + 1..];
        Line::from(vec![
            Span::styled(user, Style::default().fg(Color::Yellow)),
            Span::styled(host, Style::default().fg(Color::Cyan)),
            cursor_suffix,
        ])
    } else {
        Line::from(vec![Span::from(conn.input.as_str()), cursor_suffix])
    };
    let input_block = Block::default().borders(Borders::ALL).title(" user@host ");
    let input_paragraph = Paragraph::new(vec![input_line]).block(input_block);
    f.render_widget(input_paragraph, chunks[1]);

    // Filtered targets list
    let items: Vec<ListItem> = conn
        .filtered
        .iter()
        .enumerate()
        .map(|(i, h)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:>3} ", i + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::from(h.as_str()),
            ]))
        })
        .collect();
    let list_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" targets ({}) ", conn.filtered.len()));
    let list = List::new(items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, chunks[2], &mut state);

    // Help
    let hint = "[Enter] connect  [Tab] filter  [↑/↓] pick  /edit manage  /quit exit";
    let status = Paragraph::new(vec![Line::from(vec![
        Span::styled(" help ", Style::default().fg(Color::DarkGray)),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::from(" "),
        Span::from(hint),
    ])])
    .style(Style::default().fg(Color::Gray));
    f.render_widget(status, chunks[3]);
}

// ---------------------------------------------------------------------------
// Edit mode UI — saved sessions list with checkboxes
// ---------------------------------------------------------------------------

fn ui_edit(
    f: &mut Frame,
    app: &mut App,
    x: u16,
    full: ratatui::layout::Rect,
    width: u16,
) {
    let Mode::Edit(edit) = &mut app.mode else {
        return;
    };

    let n = app.sessions.len();
    let checked_count = edit.checked.iter().filter(|&&c| c).count();
    let list_h = n.max(3).min(20) as u16 + 2; // +2 for borders
    let total_h = 1 + list_h + 1 + 1; // title + list + selected + help
    let total_h = total_h.min(full.height.saturating_sub(2));
    let y = (full.height - total_h) / 2;
    let area = ratatui::layout::Rect::new(x, y, width, total_h);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let title = Paragraph::new(vec![Line::from(vec![
        Span::styled(
            " edit ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::from(format!(" {} saved / {} selected ", n, checked_count)),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " press Esc to exit ",
            Style::default().fg(Color::DarkGray),
        ),
    ])]);
    f.render_widget(title, chunks[0]);

    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let is_checked = *edit.checked.get(i).unwrap_or(&false);
            let checkbox = if is_checked { "[x]" } else { "[ ]" };
            ListItem::new(Line::from(vec![
                Span::styled(
                    checkbox,
                    Style::default().fg(if is_checked {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::from(" "),
                Span::from(s.as_str()),
            ]))
        })
        .collect();

    let list_block = Block::default().borders(Borders::ALL);
    let list = List::new(items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .bg(Color::Magenta)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(list, chunks[1], &mut edit.list_state);

    let sel_idx = edit.list_state.selected();
    let sel_label = if let Some(idx) = sel_idx {
        if let Some(s) = app.sessions.get(idx) {
            let is_checked = *edit.checked.get(idx).unwrap_or(&false);
            if is_checked {
                format!("{} — selected", s)
            } else {
                format!("{} — press Space to select", s)
            }
        } else {
            "(nothing saved yet)".to_string()
        }
    } else {
        "(nothing saved yet)".to_string()
    };
    let selected_line = Paragraph::new(vec![Line::from(vec![
        Span::styled(" › ", Style::default().fg(Color::Magenta)),
        Span::from(sel_label.as_str()),
    ])])
    .style(Style::default().fg(Color::Gray));
    f.render_widget(selected_line, chunks[2]);

    let status = Paragraph::new(vec![Line::from(vec![
        Span::styled(" help ", Style::default().fg(Color::DarkGray)),
        Span::styled("·", Style::default().fg(Color::DarkGray)),
        Span::from(" "),
        Span::from(app.status.as_str()),
    ])])
    .style(Style::default().fg(Color::Gray));
    f.render_widget(status, chunks[3]);
}

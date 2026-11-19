//! Embedded terminal (like Neovim's `:terminal`): runs an agent in a pseudo-terminal, keeps its
//! screen in a `vt100` parser and draws it inside the TUI.

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// What to run in a tab (kept to restart it).
#[derive(Clone, Debug)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug)]
pub struct CommandNotFound(pub String);

impl std::fmt::Display for CommandNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command not found: {}", self.0)
    }
}

impl std::error::Error for CommandNotFound {}

/// A command that will be typed into the agent once it finishes loading (e.g. `/compact`).
pub struct PendingInput {
    pub text: String,
    pub started: Instant,
    pub sent_text_at: Option<Instant>,
}

pub struct TermTab {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    pub exit_code: Option<u32>,
    out_bytes: Arc<AtomicUsize>,
    last_output_ms: Arc<AtomicU64>,
    size: (u16, u16),
    /// Lines scrolled back into history (0 = live view).
    pub scroll: usize,
    pub pending: Option<PendingInput>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// On Windows, `.cmd`/`.bat` shims (npm-installed agents) must run through `cmd.exe`.
fn resolve(launch: &Launch) -> Result<(String, Vec<String>)> {
    let path = resume_ai_core::which(&launch.program).ok_or_else(|| CommandNotFound(launch.program.clone()))?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if cfg!(windows) && (ext == "cmd" || ext == "bat") {
        let mut args = vec!["/d".to_string(), "/c".to_string(), path.display().to_string()];
        args.extend(launch.args.iter().cloned());
        return Ok(("cmd.exe".into(), args));
    }
    Ok((path.display().to_string(), launch.args.clone()))
}

/// Replies to terminal queries that programs wait for at startup (cursor position, device attributes,
/// colors). Real terminals answer these; without an answer some agents stall for seconds.
fn answer_queries(chunk: &[u8], tail: &mut Vec<u8>, parser: &Mutex<vt100::Parser>, writer: &Mutex<Box<dyn Write + Send>>) {
    let mut data = std::mem::take(tail);
    data.extend_from_slice(chunk);
    let mut replies: Vec<u8> = Vec::new();
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).filter(|w| *w == needle).count();
    for _ in 0..find(&data, b"\x1b[6n") {
        let (row, col) = parser.lock().unwrap().screen().cursor_position();
        replies.extend(format!("\x1b[{};{}R", row + 1, col + 1).bytes());
    }
    for _ in 0..(find(&data, b"\x1b[c") + find(&data, b"\x1b[0c")) {
        replies.extend(b"\x1b[?62;22c"); // VT220 with ANSI colour
    }
    for _ in 0..find(&data, b"\x1b]10;?") {
        replies.extend(b"\x1b]10;rgb:e6e6/e6e6/e6e6\x1b\\");
    }
    for _ in 0..find(&data, b"\x1b]11;?") {
        replies.extend(b"\x1b]11;rgb:1a1a/1a1a/1a1a\x1b\\");
    }
    // Keep the last bytes in case a query was split between two reads
    let keep = data.len().saturating_sub(8);
    *tail = data[keep..].to_vec();
    if !replies.is_empty() {
        let mut w = writer.lock().unwrap();
        let _ = w.write_all(&replies);
        let _ = w.flush();
    }
}

impl TermTab {
    pub fn spawn(launch: &Launch, rows: u16, cols: u16, notify: Sender<()>) -> Result<Self> {
        let (rows, cols) = (rows.max(2), cols.max(10));
        let (program, args) = resolve(launch)?;
        let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        let mut cmd = CommandBuilder::new(&program);
        cmd.args(&args);
        if let Some(cwd) = launch.cwd.as_ref().filter(|c| c.is_dir()) {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let child = pair.slave.spawn_command(cmd).with_context(|| format!("could not start {program}"))?;
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 5000)));
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(pair.master.take_writer()?));
        let out_bytes = Arc::new(AtomicUsize::new(0));
        let last_output_ms = Arc::new(AtomicU64::new(now_ms()));
        let mut reader = pair.master.try_clone_reader()?;
        {
            let (parser, writer, out_bytes, last) = (parser.clone(), writer.clone(), out_bytes.clone(), last_output_ms.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                let mut tail = Vec::new();
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            parser.lock().unwrap().process(&buf[..n]);
                            answer_queries(&buf[..n], &mut tail, &parser, &writer);
                            out_bytes.fetch_add(n, Ordering::Relaxed);
                            last.store(now_ms(), Ordering::Relaxed);
                            let _ = notify.send(());
                        }
                    }
                }
                let _ = notify.send(());
            });
        }
        Ok(TermTab {
            parser,
            writer,
            master: pair.master,
            child,
            exit_code: None,
            out_bytes,
            last_output_ms,
            size: (rows, cols),
            scroll: 0,
            pending: None,
        })
    }

    pub fn write(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn alive(&self) -> bool {
        self.exit_code.is_none()
    }

    /// Checks whether the process ended; returns true the moment it does.
    pub fn poll_exit(&mut self) -> bool {
        if self.exit_code.is_some() {
            return false;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            self.exit_code = Some(status.exit_code());
            return true;
        }
        false
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        self.parser.lock().unwrap().set_size(rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
    }

    pub fn output_bytes(&self) -> usize {
        self.out_bytes.load(Ordering::Relaxed)
    }

    pub fn quiet_for_ms(&self) -> u64 {
        now_ms().saturating_sub(self.last_output_ms.load(Ordering::Relaxed))
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let mut p = self.parser.lock().unwrap();
        let target = (self.scroll as isize + delta).max(0) as usize;
        p.set_scrollback(target);
        self.scroll = p.screen().scrollback(); // clamped to the available history
    }

    /// Mouse wheel over the terminal. Programs that asked for mouse events get the wheel as mouse events;
    /// for the rest it scrolls the history. (Never as arrow keys: agents use ↑↓ for their prompt history.)
    pub fn wheel(&mut self, up: bool, col: u16, row: u16) {
        let (mode, encoding) = {
            let p = self.parser.lock().unwrap();
            (p.screen().mouse_protocol_mode(), p.screen().mouse_protocol_encoding())
        };
        if mode == vt100::MouseProtocolMode::None {
            self.scroll_by(if up { 3 } else { -3 });
            return;
        }
        let button: u8 = if up { 64 } else { 65 };
        let bytes = match encoding {
            vt100::MouseProtocolEncoding::Sgr => format!("\x1b[<{button};{};{}M", col + 1, row + 1).into_bytes(),
            // Classic encoding: one byte per value (positions above 222 cannot be expressed)
            _ => vec![0x1b, b'[', b'M', 32 + button, 33 + col.min(222) as u8, 33 + row.min(222) as u8],
        };
        self.reset_scroll();
        self.write(&bytes);
    }

    /// Text of a visible row (to find what was clicked, e.g. "[Image #4]").
    pub fn row_text(&self, row: u16) -> String {
        let p = self.parser.lock().unwrap();
        let (_, cols) = p.screen().size();
        p.screen().rows(0, cols).nth(row as usize).unwrap_or_default()
    }

    pub fn reset_scroll(&mut self) {
        if self.scroll != 0 {
            self.scroll_by(-(self.scroll as isize));
        }
    }

    /// Sends a key to the program, encoded the way a real terminal would.
    pub fn send_key(&mut self, key: KeyEvent) {
        let app_cursor = self.parser.lock().unwrap().screen().application_cursor();
        if let Some(bytes) = encode_key(key, app_cursor) {
            self.reset_scroll();
            self.write(&bytes);
        }
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self.parser.lock().unwrap().screen().bracketed_paste();
        self.reset_scroll();
        if bracketed {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    /// Draws the terminal screen into `area`; returns the cursor position if it should be shown.
    pub fn render(&self, area: Rect, buf: &mut Buffer) -> Option<(u16, u16)> {
        let parser = self.parser.lock().unwrap();
        let screen = parser.screen();
        for row in 0..area.height {
            for col in 0..area.width {
                let Some(cell) = screen.cell(row, col) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let Some(target) = buf.cell_mut((area.x + col, area.y + row)) else { continue };
                let text = cell.contents();
                target.set_symbol(if text.is_empty() { " " } else { &text });
                target.set_style(cell_style(cell));
            }
        }
        let (r, c) = screen.cursor_position();
        (self.scroll == 0 && !screen.hide_cursor() && r < area.height && c < area.width).then_some((area.x + c, area.y + r))
    }
}

impl Drop for TermTab {
    fn drop(&mut self) {
        self.kill();
    }
}

fn convert(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default().fg(convert(cell.fgcolor())).bg(convert(cell.bgcolor()));
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    style = style.add_modifier(m);
    style
}

/// xterm encoding of a key press.
pub fn encode_key(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    let m = key.modifiers;
    let (ctrl, alt, shift) = (m.contains(KeyModifiers::CONTROL), m.contains(KeyModifiers::ALT), m.contains(KeyModifiers::SHIFT));
    // xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl
    let param = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
    let csi = |final_byte: char| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[1;{param}{final_byte}").into_bytes()
        } else if app_cursor && "ABCDHF".contains(final_byte) {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 { format!("\x1b[{n};{param}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let mut out = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let b = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 - b'a' + 1,
                    ' ' | '2' | '@' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '7' | '-' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return Some(c.to_string().into_bytes()),
                };
                vec![b]
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![if ctrl { 0x08 } else { 0x7f }],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => csi('A'),
        KeyCode::Down => csi('B'),
        KeyCode::Right => csi('C'),
        KeyCode::Left => csi('D'),
        KeyCode::Home => csi('H'),
        KeyCode::End => csi('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let c = (b'P' + n - 1) as char;
            if param > 1 { format!("\x1b[1;{param}{c}").into_bytes() } else { format!("\x1bO{c}").into_bytes() }
        }
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize]),
        _ => return None,
    };
    // Alt + plain key = ESC prefix (sequences that already carry modifiers are left alone)
    if alt && matches!(key.code, KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Esc) {
        out.insert(0, 0x1b);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> Vec<u8> {
        encode_key(KeyEvent::new(code, m), false).unwrap()
    }

    #[test]
    fn encodes_keys_like_xterm() {
        assert_eq!(k(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(k(KeyCode::Char('c'), KeyModifiers::CONTROL), vec![3]);
        assert_eq!(k(KeyCode::Char('x'), KeyModifiers::ALT), b"\x1bx");
        assert_eq!(k(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(k(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(k(KeyCode::Up, KeyModifiers::CONTROL), b"\x1b[1;5A");
        assert_eq!(k(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(k(KeyCode::F(5), KeyModifiers::NONE), b"\x1b[15~");
        assert_eq!(encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true).unwrap(), b"\x1bOA");
        assert_eq!(k(KeyCode::Char('ñ'), KeyModifiers::NONE), "ñ".as_bytes());
    }

    /// Writer that keeps what was written, to inspect the replies.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);
    impl Write for Captured {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn answers_terminal_queries_even_when_split_between_reads() {
        let parser = Mutex::new(vt100::Parser::new(10, 40, 0));
        parser.lock().unwrap().process(b"\x1b[3;7H"); // cursor at row 3, col 7
        let captured = Captured::default();
        let writer: Mutex<Box<dyn Write + Send>> = Mutex::new(Box::new(captured.clone()));
        let mut tail = Vec::new();
        answer_queries(b"hello\x1b[", &mut tail, &parser, &writer);
        answer_queries(b"6n and \x1b[c", &mut tail, &parser, &writer);
        answer_queries(b"\x1b]11;?\x07", &mut tail, &parser, &writer);
        let out = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("\x1b[3;7R"), "cursor position reply: {out:?}");
        assert!(out.contains("\x1b[?62;22c"), "device attributes reply: {out:?}");
        assert!(out.contains("\x1b]11;rgb:"), "background colour reply: {out:?}");
    }

    #[test]
    fn runs_a_program_in_a_pty() {
        let (tx, rx) = std::sync::mpsc::channel();
        let launch = if cfg!(windows) {
            Launch { program: "cmd.exe".into(), args: vec!["/d".into(), "/c".into(), "echo hello-from-pty".into()], cwd: None }
        } else {
            Launch { program: "sh".into(), args: vec!["-c".into(), "echo hello-from-pty".into()], cwd: None }
        };
        let mut tab = TermTab::spawn(&launch, 10, 60, tx).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while Instant::now() < deadline && !tab.parser.lock().unwrap().screen().contents().contains("hello-from-pty") {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(200));
        }
        assert!(tab.parser.lock().unwrap().screen().contents().contains("hello-from-pty"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10));
        tab.render(Rect::new(0, 0, 60, 10), &mut buf);
        let text: String = (0..60).flat_map(|x| (0..10).map(move |y| (x, y))).map(|p| buf[p].symbol().to_string()).collect();
        assert!(!text.trim().is_empty());
        while Instant::now() < deadline && !tab.poll_exit() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(tab.exit_code, Some(0));
    }
}

use std::{
    collections::BTreeSet,
    io::{self, IsTerminal, Read, Write},
};

use crate::config::StreamingConfig;
use crate::streaming::{AgentEventSink, AgentStreamEvent};
use unicode_width::UnicodeWidthStr;

#[cfg(unix)]
use std::{process::Command, process::Stdio};

pub fn read_user_input(prompt: &str, footer: Option<&str>) -> io::Result<Option<String>> {
    loop {
        let Some(input) = read_input_line(prompt, footer)? else {
            return Ok(None);
        };

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        return Ok(Some(input));
    }
}

fn read_input_line(prompt: &str, footer: Option<&str>) -> io::Result<Option<String>> {
    if supports_interactive_line_editor() {
        return read_input_line_interactive(prompt, footer);
    }

    read_input_line_cooked(prompt, footer)
}

fn read_input_line_cooked(prompt: &str, footer: Option<&str>) -> io::Result<Option<String>> {
    print_input_prompt(prompt, footer)?;

    let mut input = String::new();
    let bytes_read = io::stdin().read_line(&mut input)?;
    if bytes_read == 0 {
        return Ok(None);
    }

    Ok(Some(input))
}

fn print_input_prompt(prompt: &str, footer: Option<&str>) -> io::Result<()> {
    print!(
        "{}",
        render_input_prompt(prompt, footer, supports_ansi_line_clear())
    );

    io::stdout().flush()?;
    Ok(())
}

fn render_input_prompt(prompt: &str, footer: Option<&str>, ansi_line_clear: bool) -> String {
    let mut output = String::new();
    let clear_line = if ansi_line_clear { "\x1b[2K" } else { "" };

    if let Some(footer) = footer.filter(|footer| !footer.is_empty()) {
        output.push_str(clear_line);
        output.push_str(footer);
        output.push('\n');
    }

    output.push_str(clear_line);
    output.push_str(prompt);
    output
}

fn supports_ansi_line_clear() -> bool {
    io::stdout().is_terminal()
}

fn supports_interactive_line_editor() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

#[cfg(unix)]
fn read_input_line_interactive(prompt: &str, footer: Option<&str>) -> io::Result<Option<String>> {
    print_input_prompt(prompt, footer)?;
    let _raw_mode = RawModeGuard::enable()?;
    let mut line = EditableLine::default();
    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    loop {
        match read_input_key(&mut stdin)? {
            InputKey::Enter => {
                print!("\r\n");
                io::stdout().flush()?;
                return Ok(Some(line.into_string()));
            }
            InputKey::Backspace => {
                if line.backspace() {
                    redraw_editable_line(prompt, &line)?;
                }
            }
            InputKey::Delete => {
                if line.delete() {
                    redraw_editable_line(prompt, &line)?;
                }
            }
            InputKey::Left => {
                if line.move_left() {
                    redraw_editable_line(prompt, &line)?;
                }
            }
            InputKey::Right => {
                if line.move_right() {
                    redraw_editable_line(prompt, &line)?;
                }
            }
            InputKey::Text(text) => {
                line.insert_str(&text);
                redraw_editable_line(prompt, &line)?;
            }
            InputKey::CtrlD => {
                if line.as_str().is_empty() {
                    print!("\r\n");
                    io::stdout().flush()?;
                    return Ok(None);
                }
            }
            InputKey::CtrlC => {
                print!("^C\r\n");
                io::stdout().flush()?;
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "input interrupted",
                ));
            }
            InputKey::Ignore => {}
        }
    }
}

#[cfg(not(unix))]
fn read_input_line_interactive(prompt: &str, footer: Option<&str>) -> io::Result<Option<String>> {
    read_input_line_cooked(prompt, footer)
}

#[cfg(unix)]
struct RawModeGuard {
    original: libc::termios,
}

#[cfg(unix)]
impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        let fd = libc::STDIN_FILENO;
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();

        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let original = unsafe { original.assume_init() };
        let mut raw = original;
        unsafe {
            libc::cfmakeraw(&mut raw);
        }

        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self { original })
    }
}

#[cfg(unix)]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
    }
}

#[derive(Debug, Default)]
struct EditableLine {
    text: String,
    cursor: usize,
}

impl EditableLine {
    fn as_str(&self) -> &str {
        &self.text
    }

    fn into_string(self) -> String {
        self.text
    }

    fn insert_str(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    fn backspace(&mut self) -> bool {
        let Some(previous) = self.previous_char_boundary() else {
            return false;
        };

        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        true
    }

    fn delete(&mut self) -> bool {
        if self.cursor >= self.text.len() {
            return false;
        }

        let next = self.next_char_boundary();
        self.text.replace_range(self.cursor..next, "");
        true
    }

    fn move_left(&mut self) -> bool {
        let Some(previous) = self.previous_char_boundary() else {
            return false;
        };

        self.cursor = previous;
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.text.len() {
            return false;
        }

        self.cursor = self.next_char_boundary();
        true
    }

    fn cursor_display_width(&self, prompt: &str) -> usize {
        UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(&self.text[..self.cursor])
    }

    fn end_display_width(&self, prompt: &str) -> usize {
        UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(self.text.as_str())
    }

    fn previous_char_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
    }

    fn next_char_boundary(&self) -> usize {
        self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(index, _)| self.cursor + index)
            .unwrap_or(self.text.len())
    }
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum InputKey {
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Text(String),
    CtrlD,
    CtrlC,
    Ignore,
}

#[cfg(unix)]
fn read_input_key(input: &mut impl Read) -> io::Result<InputKey> {
    let Some(byte) = read_byte(input)? else {
        return Ok(InputKey::CtrlD);
    };

    match byte {
        b'\r' | b'\n' => Ok(InputKey::Enter),
        0x03 => Ok(InputKey::CtrlC),
        0x04 => Ok(InputKey::CtrlD),
        0x08 | 0x7f => Ok(InputKey::Backspace),
        0x1b => read_escape_key(input),
        0x00..=0x1f => Ok(InputKey::Ignore),
        0x20..=0x7e => Ok(InputKey::Text((byte as char).to_string())),
        _ => read_utf8_key(input, byte),
    }
}

#[cfg(unix)]
fn read_escape_key(input: &mut impl Read) -> io::Result<InputKey> {
    let Some(first) = read_byte(input)? else {
        return Ok(InputKey::Ignore);
    };
    if first != b'[' {
        return Ok(InputKey::Ignore);
    }

    let Some(second) = read_byte(input)? else {
        return Ok(InputKey::Ignore);
    };

    match second {
        b'C' => Ok(InputKey::Right),
        b'D' => Ok(InputKey::Left),
        b'3' => {
            let _ = read_byte(input)?;
            Ok(InputKey::Delete)
        }
        _ => Ok(InputKey::Ignore),
    }
}

#[cfg(unix)]
fn read_utf8_key(input: &mut impl Read, first: u8) -> io::Result<InputKey> {
    let width = utf8_sequence_width(first);
    if width == 0 {
        return Ok(InputKey::Ignore);
    }

    let mut bytes = vec![first];
    for _ in 1..width {
        let Some(byte) = read_byte(input)? else {
            return Ok(InputKey::Ignore);
        };
        bytes.push(byte);
    }

    match String::from_utf8(bytes) {
        Ok(text) => Ok(InputKey::Text(text)),
        Err(_) => Ok(InputKey::Ignore),
    }
}

#[cfg(unix)]
fn read_byte(input: &mut impl Read) -> io::Result<Option<u8>> {
    let mut byte = [0];
    match input.read(&mut byte)? {
        0 => Ok(None),
        _ => Ok(Some(byte[0])),
    }
}

#[cfg(unix)]
fn utf8_sequence_width(first: u8) -> usize {
    match first {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 0,
    }
}

fn redraw_editable_line(prompt: &str, line: &EditableLine) -> io::Result<()> {
    print!("{}", render_editable_line(prompt, line));
    io::stdout().flush()
}

fn render_editable_line(prompt: &str, line: &EditableLine) -> String {
    let mut output = format!("\r\x1b[2K{prompt}{}", line.as_str());
    let cursor_width = line.cursor_display_width(prompt);
    if cursor_width != line.end_display_width(prompt) {
        output.push('\r');
        if cursor_width > 0 {
            output.push_str(&format!("\x1b[{cursor_width}C"));
        }
    }
    output
}

pub fn is_exit_command(input: &str) -> bool {
    input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit")
}

pub fn read_secret_input(prompt: &str) -> io::Result<Option<String>> {
    print!("{prompt}");
    io::stdout().flush()?;

    let echo_disabled = set_stdin_echo(false).unwrap_or(false);

    let mut input = String::new();
    let read_result = io::stdin().read_line(&mut input);

    if echo_disabled {
        let _ = set_stdin_echo(true);
        println!();
    }

    if read_result? == 0 {
        return Ok(None);
    }

    Ok(Some(input.trim().to_string()))
}

#[cfg(unix)]
fn set_stdin_echo(enabled: bool) -> io::Result<bool> {
    let arg = if enabled { "echo" } else { "-echo" };
    let status = Command::new("stty")
        .arg(arg)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    Ok(status.success())
}

#[cfg(not(unix))]
fn set_stdin_echo(_enabled: bool) -> io::Result<bool> {
    Ok(false)
}

// ── Console trace renderer ────────────────────────────────────────────

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub struct ConsoleTraceRenderer {
    config: StreamingConfig,
    is_tty: bool,
    printed_thinking_header: bool,
    printed_reasoning_delta: bool,
    printed_answer_header: bool,
    printed_tool_calls: BTreeSet<u32>,
    open_tool_call_line: bool,
}

impl ConsoleTraceRenderer {
    pub fn new(config: &StreamingConfig) -> Self {
        Self {
            config: config.clone(),
            is_tty: io::stdout().is_terminal(),
            printed_thinking_header: false,
            printed_reasoning_delta: false,
            printed_answer_header: false,
            printed_tool_calls: BTreeSet::new(),
            open_tool_call_line: false,
        }
    }

    fn finish_reasoning_line(&mut self) {
        if self.printed_reasoning_delta {
            println!();
            self.printed_reasoning_delta = false;
        }
    }
}

impl AgentEventSink for ConsoleTraceRenderer {
    fn on_event(&mut self, event: &AgentStreamEvent) -> anyhow::Result<()> {
        match event {
            AgentStreamEvent::ResponseStarted { round: _ } => {
                self.printed_thinking_header = false;
                self.printed_reasoning_delta = false;
                self.printed_answer_header = false;
                self.printed_tool_calls.clear();
                self.open_tool_call_line = false;
            }

            AgentStreamEvent::ReasoningStarted => {
                if self.config.show_reasoning && !self.printed_thinking_header {
                    if self.is_tty {
                        print!("{DIM}thinking> {RESET}");
                    } else {
                        print!("thinking> ");
                    }
                    io::stdout().flush()?;
                    self.printed_thinking_header = true;
                }
            }

            AgentStreamEvent::ReasoningDelta(text) => {
                if self.config.show_reasoning {
                    if self.is_tty {
                        print!("{DIM}{text}{RESET}");
                    } else {
                        print!("{text}");
                    }
                    self.printed_reasoning_delta = true;
                    io::stdout().flush()?;
                }
            }

            AgentStreamEvent::AnswerStarted => {
                self.finish_reasoning_line();
                if !self.printed_answer_header {
                    print!("agent> ");
                    io::stdout().flush()?;
                    self.printed_answer_header = true;
                }
            }

            AgentStreamEvent::AnswerDelta(text) => {
                print!("{text}");
                io::stdout().flush()?;
            }

            AgentStreamEvent::ToolCallDelta {
                index,
                name,
                arguments_delta,
                ..
            } => {
                if self.config.show_tool_call_deltas {
                    if name.is_none() && arguments_delta.is_none() {
                        return Ok(());
                    }

                    self.finish_reasoning_line();

                    if self.printed_tool_calls.insert(*index) {
                        if self.open_tool_call_line {
                            println!();
                        }

                        match name.as_deref() {
                            Some(name) => print!("tool[{index}]> {name} "),
                            None => print!("tool[{index}]> "),
                        }
                        self.open_tool_call_line = true;
                    }

                    if let Some(args) = arguments_delta {
                        print!("{args}");
                    }

                    io::stdout().flush()?;
                }
            }

            AgentStreamEvent::ResponseFinished { finish_reason: _ } => {
                if self.printed_answer_header
                    || self.printed_reasoning_delta
                    || self.open_tool_call_line
                {
                    println!();
                }
                self.printed_reasoning_delta = false;
                self.open_tool_call_line = false;
            }

            AgentStreamEvent::Usage(_) => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_prompt_does_not_place_footer_under_editing_line() {
        let output = render_input_prompt(">>> ", Some("context> 10 / 100 tokens"), true);

        assert_eq!(output, "\x1b[2Kcontext> 10 / 100 tokens\n\x1b[2K>>> ");
        assert!(!output.contains("\x1b[1A"));
        assert!(!output.contains("\x1b[4C"));
    }

    #[test]
    fn tty_prompt_without_footer_keeps_single_clean_prompt_line() {
        let output = render_input_prompt(">>> ", None, true);

        assert_eq!(output, "\x1b[2K>>> ");
    }

    #[test]
    fn non_tty_prompt_avoids_ansi_sequences() {
        let output = render_input_prompt(">>> ", Some("context> 10 / 100 tokens"), false);

        assert_eq!(output, "context> 10 / 100 tokens\n>>> ");
    }

    #[test]
    fn editable_line_backspace_removes_a_complete_chinese_character() {
        let mut line = EditableLine::default();
        line.insert_str("中文");

        assert!(line.backspace());

        assert_eq!(line.as_str(), "中");
        assert_eq!(line.cursor_display_width(">>> "), 6);
    }

    #[test]
    fn redraw_line_clears_and_repositions_by_display_width() {
        let mut line = EditableLine::default();
        line.insert_str("中a");
        line.move_left();

        let output = render_editable_line(">>> ", &line);

        assert_eq!(output, "\r\x1b[2K>>> 中a\r\x1b[6C");
    }
}

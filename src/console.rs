use std::io::{self, IsTerminal, Write};

#[cfg(unix)]
use std::{process::Command, process::Stdio};

pub fn read_user_input(prompt: &str, footer: Option<&str>) -> io::Result<Option<String>> {
    loop {
        let inline_footer = print_input_prompt(prompt, footer)?;

        let mut input = String::new();
        let bytes_read = io::stdin().read_line(&mut input)?;

        if inline_footer {
            println!();
        } else if let Some(footer) = footer.filter(|footer| !footer.is_empty()) {
            println!("{footer}");
        }

        if bytes_read == 0 {
            return Ok(None);
        }

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        return Ok(Some(input));
    }
}

fn print_input_prompt(prompt: &str, footer: Option<&str>) -> io::Result<bool> {
    let footer = footer.filter(|footer| !footer.is_empty());
    let inline_footer = footer.is_some() && supports_inline_footer();

    if inline_footer {
        let footer = footer.expect("footer was checked above");
        print!("{prompt}\n{footer}\x1b[1A\r");
        let prompt_width = prompt.chars().count();
        if prompt_width > 0 {
            print!("\x1b[{prompt_width}C");
        }
    } else {
        print!("{prompt}");
    }

    io::stdout().flush()?;
    Ok(inline_footer)
}

fn supports_inline_footer() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
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

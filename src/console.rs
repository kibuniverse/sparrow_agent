use std::io::{self, Write};

#[cfg(unix)]
use std::{process::Command, process::Stdio};

pub fn read_user_input(prompt: &str) -> io::Result<Option<String>> {
    loop {
        print!("{prompt}");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            return Ok(None);
        }

        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        return Ok(Some(input));
    }
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

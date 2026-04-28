use std::io::{self, Write};

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

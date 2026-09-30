//! Interactive yes/no confirmation for destructive operations.

use std::io::{BufRead, Write};

/// Writes `prompt` to `output` and reads one line from `input`. Returns
/// true only for an explicit `y`/`yes` (case-insensitive); anything else,
/// including EOF (no terminal attached), is a no.
pub fn confirm_from<R: BufRead, W: Write>(prompt: &str, input: &mut R, output: &mut W) -> std::io::Result<bool> {
    write!(output, "{prompt} [y/N] ")?;
    output.flush()?;

    let mut line = String::new();
    input.read_line(&mut line)?;

    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

/// Asks on stderr and reads the answer from stdin.
pub fn confirm(prompt: &str) -> std::io::Result<bool> {
    confirm_from(prompt, &mut std::io::stdin().lock(), &mut std::io::stderr())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(answer: &str) -> bool {
        let mut output = Vec::new();
        confirm_from("Delete?", &mut answer.as_bytes(), &mut output).unwrap()
    }

    #[test]
    fn accepts_y_and_yes_case_insensitively() {
        assert!(ask("y\n"));
        assert!(ask("YES\n"));
        assert!(ask("  Yes  \n"));
    }

    #[test]
    fn rejects_everything_else() {
        assert!(!ask("n\n"));
        assert!(!ask("\n"));
        assert!(!ask("maybe\n"));
    }

    #[test]
    fn rejects_on_eof() {
        assert!(!ask(""));
    }

    #[test]
    fn writes_the_prompt() {
        let mut output = Vec::new();
        confirm_from("Delete 3 files?", &mut "n\n".as_bytes(), &mut output).unwrap();

        assert_eq!(String::from_utf8(output).unwrap(), "Delete 3 files? [y/N] ");
    }
}

//! Terminal output and questions.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Result, bail};

fn styled(code: &str, text: &str) -> String {
    if std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// A step of the installation, in bold.
pub fn step(msg: impl AsRef<str>) {
    eprintln!("{} {}", styled("1;36", "==>"), styled("1", msg.as_ref()));
}

/// A detail under the current step.
pub fn info(msg: impl AsRef<str>) {
    eprintln!("    {}", msg.as_ref());
}

/// Something the user should know about, but not an error.
pub fn warn(msg: impl AsRef<str>) {
    eprintln!("{} {}", styled("1;33", "warning:"), msg.as_ref());
}

/// Asks a yes/no question (default yes). `assume_yes` (`-y`) answers it without asking; without a
/// terminal to ask on, the answer must come from `-y`.
pub fn confirm(question: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!("{question}\nNo terminal to answer on: run again with -y to accept.");
    }
    eprint!("{} {question} [Y/n] ", styled("1;36", "?"));
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "" | "y" | "yes"))
}

/// Download progress on one line, redrawn in place (only on a terminal).
pub struct Progress {
    label: String,
    total: Option<u64>,
    done: u64,
    shown: u64,
    tty: bool,
}

impl Progress {
    /// Starts a progress line for `label`; `total` is the size when known.
    pub fn new(label: &str, total: Option<u64>) -> Self {
        Self { label: label.to_string(), total, done: 0, shown: 0, tty: std::io::stderr().is_terminal() }
    }

    /// Adds `n` bytes.
    pub fn add(&mut self, n: u64) {
        self.done += n;
        if self.tty && self.done - self.shown >= 256 * 1024 {
            self.shown = self.done;
            self.draw();
        }
    }

    fn draw(&self) {
        let mb = self.done as f64 / 1_048_576.0;
        match self.total {
            Some(t) if t > 0 => eprint!("\r    {} {:.1} / {:.1} MB ({:.0}%)  ", self.label, mb, t as f64 / 1_048_576.0, self.done as f64 * 100.0 / t as f64),
            _ => eprint!("\r    {} {:.1} MB  ", self.label, mb),
        }
        std::io::stderr().flush().ok();
    }

    /// Ends the line.
    pub fn finish(self) {
        if self.tty {
            self.draw();
            eprintln!();
        } else {
            info(format!("{} {:.1} MB", self.label, self.done as f64 / 1_048_576.0));
        }
    }
}

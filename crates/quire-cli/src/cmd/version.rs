//! The `quire version`, `quire exit-codes` and `quire help` commands (owner: W8)
//!
//! W0b: `exit-codes`.

use std::fmt::Write;

use quire_api::cli::ExitCode;

use crate::output;

/// `quire exit-codes`: the §7.7 table. `--json` prints `[{"code": 0, "name": "success", "meaning": "Success"}, …]`;
/// without it, one line per code with its name, meaning and typical causes.
pub fn exit_codes(json: bool) -> ExitCode {
    if json {
        output::print_json(&ExitCode::table())
    } else {
        output::print_text(&exit_codes_text())
    }
}

fn exit_codes_text() -> String {
    let width = ExitCode::ALL
        .iter()
        .map(|c| c.name().len())
        .max()
        .unwrap_or(0);
    let mut text = String::new();
    for code in ExitCode::ALL {
        let _ = write!(
            text,
            "{:>2}  {:<width$}  {}",
            code.code(),
            code.name(),
            code.meaning()
        );
        if !code.causes().is_empty() {
            let _ = write!(text, ": {}", code.causes());
        }
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_table_has_one_line_per_code() {
        let text = exit_codes_text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), ExitCode::ALL.len());
        assert_eq!(lines[0].trim_end(), " 0  success              Success");
        assert!(
            lines[9].starts_with(" 9  precondition_failed  Precondition failed: 412 rev_mismatch"),
            "{}",
            lines[9]
        );
        assert!(lines[12].starts_with("12  server_error"), "{}", lines[12]);
    }
}

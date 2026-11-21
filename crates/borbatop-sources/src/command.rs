//! Running external commands with a bounded, typed result.

use std::process::Command;

use crate::error::CommandError;

const STDERR_CONTEXT_CHARS: usize = 200;

/// Runs `program` and returns its standard output as lossy UTF-8.
///
/// # Arguments
///
/// * `program` - executable looked up in `PATH`
/// * `arguments` - arguments, passed without a shell
/// * `accepted_empty_codes` - exit codes that mean "nothing found" when standard output is empty
///   (`pacman -Qu` exits with 1 when there is no upgrade)
///
/// # Errors
///
/// * [`CommandError::Spawn`] when the program cannot be started.
/// * [`CommandError::Failed`] when it exits with any other non-zero status.
pub(crate) fn run(
    program: &'static str,
    arguments: &[&str],
    accepted_empty_codes: &[i32],
) -> Result<String, CommandError> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|source| CommandError::Spawn { program, source })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    let code = output.status.code();
    let is_empty_result =
        stdout.trim().is_empty() && code.is_some_and(|code| accepted_empty_codes.contains(&code));
    if output.status.success() || is_empty_result {
        return Ok(stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(STDERR_CONTEXT_CHARS)
        .collect();
    Err(CommandError::Failed {
        program,
        code,
        stderr,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn returns_stdout_on_success() {
        assert_eq!(run("echo", &["hi"], &[]).unwrap(), "hi\n");
    }

    #[test]
    fn accepted_code_with_empty_output_is_an_empty_result() {
        assert_eq!(run("false", &[], &[1]).unwrap(), "");
    }

    #[test]
    fn unaccepted_failure_reports_the_exit_code() {
        let error = run("false", &[], &[]).unwrap_err();
        assert!(matches!(error, CommandError::Failed { code: Some(1), .. }));
    }

    #[test]
    fn missing_program_is_a_spawn_error() {
        let error = run("borbatop-no-such-program", &[], &[]).unwrap_err();
        assert!(matches!(error, CommandError::Spawn { .. }));
    }
}

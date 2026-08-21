//! Per-command closed grammar for the kimi terminal tool.
//!
//! The kimi ACP server sends a single `command` string (for example
//! `git status` or `rg -n 'foo' -- src`). This module tokenizes that string
//! with shell-like quoting (without any shell operators or expansion) and
//! validates it against a fixed per-command template. The caller then builds
//! the final argv itself: the model never supplies positional arguments
//! separately and never controls which binary is executed.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    Git,
    Rg,
    Find,
    Sed,
    Grep,
    Cat,
    Ls,
    Head,
    Tail,
    Wc,
}

impl Binary {
    pub fn name(self) -> &'static str {
        match self {
            Binary::Git => "git",
            Binary::Rg => "rg",
            Binary::Find => "find",
            Binary::Sed => "sed",
            Binary::Grep => "grep",
            Binary::Cat => "cat",
            Binary::Ls => "ls",
            Binary::Head => "head",
            Binary::Tail => "tail",
            Binary::Wc => "wc",
        }
    }

    fn parse(name: &str) -> Option<Binary> {
        Some(match name {
            "git" => Binary::Git,
            "rg" => Binary::Rg,
            "find" => Binary::Find,
            "sed" => Binary::Sed,
            "grep" => Binary::Grep,
            "cat" => Binary::Cat,
            "ls" => Binary::Ls,
            "head" => Binary::Head,
            "tail" => Binary::Tail,
            "wc" => Binary::Wc,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub binary: Binary,
    /// Full argv including the binary name. The caller resolves the binary to
    /// a trusted absolute path and prepends git hardening arguments.
    pub argv: Vec<String>,
    /// Path operands that must be verified inside the authorized root with
    /// no-follow semantics before execution. Patterns, refs, scripts and
    /// globs are intentionally absent.
    pub path_operands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrammarError {
    EmptyCommand,
    Tokenize(String),
    UnknownCommand(String),
    ForbiddenOption {
        command: &'static str,
        option: String,
    },
    UnexpectedToken {
        command: &'static str,
        token: String,
    },
    MissingOperand {
        command: &'static str,
        what: &'static str,
    },
    NonNumeric {
        command: &'static str,
        value: String,
    },
    InvalidScript {
        command: &'static str,
        script: String,
    },
}

impl fmt::Display for GrammarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrammarError::EmptyCommand => write!(formatter, "terminal command is empty"),
            GrammarError::Tokenize(detail) => {
                write!(formatter, "terminal command syntax: {detail}")
            }
            GrammarError::UnknownCommand(command) => {
                write!(formatter, "terminal command is not allowed: {command}")
            }
            GrammarError::ForbiddenOption { command, option } => {
                write!(
                    formatter,
                    "terminal command {command}: option is not allowed: {option}"
                )
            }
            GrammarError::UnexpectedToken { command, token } => {
                write!(
                    formatter,
                    "terminal command {command}: unexpected token: {token}"
                )
            }
            GrammarError::MissingOperand { command, what } => {
                write!(formatter, "terminal command {command}: missing {what}")
            }
            GrammarError::NonNumeric { command, value } => {
                write!(
                    formatter,
                    "terminal command {command}: expected a number, got: {value}"
                )
            }
            GrammarError::InvalidScript { command, script } => {
                write!(
                    formatter,
                    "terminal command {command}: sed script is not allowed: {script}"
                )
            }
        }
    }
}

impl std::error::Error for GrammarError {}

/// Shell-like tokenizer that supports single quotes, double quotes and
/// backslash escapes but rejects every shell operator (redirection, pipes,
/// command substitution, backgrounding). It performs no expansion.
fn tokenize(command: &str) -> Result<Vec<String>, GrammarError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut in_token = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(other) => current.push(other),
                        None => {
                            return Err(GrammarError::Tokenize("unterminated single quote".into()));
                        }
                    }
                }
            }
            '"' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('"') | Some('\\') => current.push('\\'),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => {
                                return Err(GrammarError::Tokenize(
                                    "unterminated double quote".into(),
                                ));
                            }
                        },
                        Some(other) => current.push(other),
                        None => {
                            return Err(GrammarError::Tokenize("unterminated double quote".into()));
                        }
                    }
                }
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    Some(escaped) => current.push(escaped),
                    None => return Err(GrammarError::Tokenize("trailing backslash".into())),
                }
            }
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            '>' | '<' | '|' | ';' | '&' | '`' | '$' => {
                return Err(GrammarError::Tokenize(format!(
                    "shell operator is not allowed: {ch}"
                )));
            }
            other => {
                in_token = true;
                current.push(other);
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return Err(GrammarError::EmptyCommand);
    }
    Ok(tokens)
}

fn is_option(token: &str) -> bool {
    token.starts_with('-') && token != "-"
}

fn is_number(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit())
}

/// A path operand must never begin with `-` unless it sits after a fixed
/// `--` separator, must never be absolute, and must never contain a `..`
/// component.
fn reject_absolute_or_option_operand(
    token: &str,
    command: &'static str,
    after_separator: bool,
) -> Result<(), GrammarError> {
    if token.starts_with('/') {
        return Err(GrammarError::UnexpectedToken {
            command,
            token: token.to_string(),
        });
    }
    if has_parent_traversal(token) {
        return Err(GrammarError::UnexpectedToken {
            command,
            token: token.to_string(),
        });
    }
    if !after_separator && is_option(token) {
        return Err(GrammarError::ForbiddenOption {
            command,
            option: token.to_string(),
        });
    }
    Ok(())
}

fn has_parent_traversal(token: &str) -> bool {
    token.split('/').any(|component| component == "..")
}

pub fn parse_command(command: &str) -> Result<ParsedCommand, GrammarError> {
    let tokens = tokenize(command)?;
    let binary =
        Binary::parse(&tokens[0]).ok_or_else(|| GrammarError::UnknownCommand(tokens[0].clone()))?;
    parse_binary(binary, &tokens[1..])
}

fn parse_binary(binary: Binary, tokens: &[String]) -> Result<ParsedCommand, GrammarError> {
    let command = binary.name();
    let mut argv = vec![binary.name().to_string()];
    let mut path_operands = Vec::new();

    match binary {
        Binary::Git => parse_git(command, tokens, &mut argv, &mut path_operands)?,
        Binary::Rg => parse_rg(command, tokens, &mut argv, &mut path_operands)?,
        Binary::Find => parse_find(command, tokens, &mut argv, &mut path_operands)?,
        Binary::Sed => parse_sed(command, tokens, &mut argv, &mut path_operands)?,
        Binary::Grep => parse_grep(command, tokens, &mut argv, &mut path_operands)?,
        Binary::Cat | Binary::Ls | Binary::Wc => {
            parse_plain_paths(command, tokens, &mut argv, &mut path_operands)?;
        }
        Binary::Head | Binary::Tail => {
            parse_head_tail(command, tokens, &mut argv, &mut path_operands)?;
        }
    }

    Ok(ParsedCommand {
        binary,
        argv,
        path_operands,
    })
}

fn parse_git(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    let Some(subcommand) = tokens.first() else {
        return Err(GrammarError::MissingOperand {
            command,
            what: "git subcommand",
        });
    };
    // The subcommand itself is never an option; anything else that starts
    // with `-` here is a forbidden git option (including -C/--git-dir/-c/...).
    if is_option(subcommand) {
        return Err(GrammarError::ForbiddenOption {
            command,
            option: subcommand.clone(),
        });
    }
    argv.push(subcommand.clone());

    let rest = &tokens[1..];
    match subcommand.as_str() {
        "status" => {
            for token in rest {
                match token.as_str() {
                    "--short" => argv.push(token.clone()),
                    _ if is_option(token) => {
                        return Err(GrammarError::ForbiddenOption {
                            command,
                            option: token.clone(),
                        });
                    }
                    _ => {
                        reject_absolute_or_option_operand(token, command, false)?;
                        path_operands.push(token.clone());
                        argv.push(token.clone());
                    }
                }
            }
        }
        "log" => {
            parse_git_log(command, rest, argv, path_operands)?;
        }
        "diff" => {
            parse_git_diff(command, rest, argv, path_operands)?;
        }
        "show" => {
            parse_git_show(command, rest, argv, path_operands)?;
        }
        other => {
            return Err(GrammarError::UnexpectedToken {
                command,
                token: other.to_string(),
            });
        }
    }
    Ok(())
}

fn parse_git_log(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    let mut index = 0;
    let mut seen_separator = false;
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "--oneline" if !seen_separator => argv.push(token.clone()),
            "-n" if !seen_separator => {
                let value = tokens.get(index + 1).ok_or(GrammarError::MissingOperand {
                    command,
                    what: "count after -n",
                })?;
                if !is_number(value) {
                    return Err(GrammarError::NonNumeric {
                        command,
                        value: value.clone(),
                    });
                }
                argv.push(token.clone());
                argv.push(value.clone());
                index += 1;
            }
            "--" if !seen_separator => {
                seen_separator = true;
                argv.push(token.clone());
            }
            _ if !seen_separator && is_option(token) => {
                return Err(GrammarError::ForbiddenOption {
                    command,
                    option: token.clone(),
                });
            }
            _ => {
                reject_absolute_or_option_operand(token, command, seen_separator)?;
                path_operands.push(token.clone());
                argv.push(token.clone());
            }
        }
        index += 1;
    }
    Ok(())
}

fn parse_git_diff(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    let mut index = 0;
    let mut seen_separator = false;
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "--stat" | "--name-only" if !seen_separator => argv.push(token.clone()),
            "--" if !seen_separator => {
                seen_separator = true;
                argv.push(token.clone());
            }
            _ if !seen_separator && is_option(token) => {
                return Err(GrammarError::ForbiddenOption {
                    command,
                    option: token.clone(),
                });
            }
            _ => {
                reject_absolute_or_option_operand(token, command, seen_separator)?;
                path_operands.push(token.clone());
                argv.push(token.clone());
            }
        }
        index += 1;
    }
    Ok(())
}

fn parse_git_show(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    let mut index = 0;
    let mut seen_separator = false;
    let mut seen_ref = false;
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "--" if !seen_separator => {
                seen_separator = true;
                argv.push(token.clone());
            }
            _ if !seen_separator && is_option(token) => {
                return Err(GrammarError::ForbiddenOption {
                    command,
                    option: token.clone(),
                });
            }
            _ if !seen_separator && !seen_ref => {
                // A single ref operand before `--` is permitted and is not a path.
                reject_absolute_or_option_operand(token, command, false)?;
                argv.push(token.clone());
                seen_ref = true;
            }
            _ => {
                reject_absolute_or_option_operand(token, command, seen_separator)?;
                path_operands.push(token.clone());
                argv.push(token.clone());
            }
        }
        index += 1;
    }
    Ok(())
}

fn parse_rg(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    // rg [-n|-l|-i|--no-heading] [-g <glob>] -- <pattern> <path>...
    let mut index = 0;
    let mut seen_separator = false;
    let mut seen_flags = std::collections::BTreeSet::new();
    while index < tokens.len() && !seen_separator {
        let token = &tokens[index];
        match token.as_str() {
            "-n" | "-l" | "-i" | "--no-heading" => {
                if !seen_flags.insert(token.clone()) {
                    return Err(GrammarError::UnexpectedToken {
                        command,
                        token: token.clone(),
                    });
                }
                argv.push(token.clone());
            }
            "-g" => {
                let glob = tokens.get(index + 1).ok_or(GrammarError::MissingOperand {
                    command,
                    what: "glob after -g",
                })?;
                if is_option(glob) {
                    return Err(GrammarError::ForbiddenOption {
                        command,
                        option: glob.clone(),
                    });
                }
                argv.push(token.clone());
                argv.push(glob.clone());
                index += 1;
            }
            "--" => {
                seen_separator = true;
                argv.push(token.clone());
            }
            _ if is_option(token) => {
                return Err(GrammarError::ForbiddenOption {
                    command,
                    option: token.clone(),
                });
            }
            _ => {
                return Err(GrammarError::UnexpectedToken {
                    command,
                    token: token.clone(),
                });
            }
        }
        index += 1;
    }
    if !seen_separator {
        return Err(GrammarError::MissingOperand {
            command,
            what: "'--' separator",
        });
    }
    let pattern = tokens.get(index).ok_or(GrammarError::MissingOperand {
        command,
        what: "search pattern",
    })?;
    // After `--` a leading dash is a literal pattern, so no option check.
    if pattern.starts_with('/') {
        return Err(GrammarError::UnexpectedToken {
            command,
            token: pattern.clone(),
        });
    }
    argv.push(pattern.clone());
    index += 1;
    if index >= tokens.len() {
        return Err(GrammarError::MissingOperand {
            command,
            what: "path",
        });
    }
    for path in &tokens[index..] {
        if path.starts_with('/') {
            return Err(GrammarError::UnexpectedToken {
                command,
                token: path.clone(),
            });
        }
        path_operands.push(path.clone());
        argv.push(path.clone());
    }
    Ok(())
}

fn parse_find(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    let Some(path) = tokens.first() else {
        return Err(GrammarError::MissingOperand {
            command,
            what: "path",
        });
    };
    reject_absolute_or_option_operand(path, command, false)?;
    path_operands.push(path.clone());
    argv.push(path.clone());

    let mut index = 1;
    let mut seen = std::collections::BTreeSet::new();
    while index < tokens.len() {
        let token = &tokens[index];
        match token.as_str() {
            "-print" => {
                if !seen.insert("print") {
                    return Err(GrammarError::UnexpectedToken {
                        command,
                        token: token.clone(),
                    });
                }
                argv.push(token.clone());
            }
            "-name" | "-type" | "-maxdepth" | "-mindepth" => {
                if !seen.insert(token.as_str()) {
                    return Err(GrammarError::UnexpectedToken {
                        command,
                        token: token.clone(),
                    });
                }
                let value = tokens.get(index + 1).ok_or(GrammarError::MissingOperand {
                    command,
                    what: "predicate value",
                })?;
                if token == "-maxdepth" || token == "-mindepth" {
                    if !is_number(value) {
                        return Err(GrammarError::NonNumeric {
                            command,
                            value: value.clone(),
                        });
                    }
                } else if token == "-type" && !is_single_char(value) {
                    return Err(GrammarError::UnexpectedToken {
                        command,
                        token: value.clone(),
                    });
                } else if token == "-name" && is_option(value) {
                    return Err(GrammarError::ForbiddenOption {
                        command,
                        option: value.clone(),
                    });
                }
                argv.push(token.clone());
                argv.push(value.clone());
                index += 1;
            }
            _ => {
                return Err(GrammarError::ForbiddenOption {
                    command,
                    option: token.clone(),
                });
            }
        }
        index += 1;
    }
    Ok(())
}

fn is_single_char(value: &str) -> bool {
    value.chars().count() == 1
}

fn parse_sed(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    if tokens.len() != 3 {
        return Err(GrammarError::UnexpectedToken {
            command,
            token: tokens.get(2).cloned().unwrap_or_default(),
        });
    }
    let flag = &tokens[0];
    let script = &tokens[1];
    let file = &tokens[2];
    if flag != "-n" {
        return Err(GrammarError::ForbiddenOption {
            command,
            option: flag.clone(),
        });
    }
    validate_sed_script(command, script)?;
    reject_absolute_or_option_operand(file, command, false)?;
    argv.push(flag.clone());
    argv.push(script.clone());
    path_operands.push(file.clone());
    argv.push(file.clone());
    Ok(())
}

fn validate_sed_script(command: &'static str, script: &str) -> Result<(), GrammarError> {
    if script.is_empty() {
        return Err(GrammarError::MissingOperand {
            command,
            what: "sed script",
        });
    }
    // Only addresses (numbers, `$`, ranges with `,`) and the instructions
    // p/=/d are permitted, separated by `;` or newlines. Any other character
    // (including e/r/w, `!`, `/` regex addresses) is rejected.
    for ch in script.chars() {
        let allowed = ch.is_ascii_digit()
            || matches!(ch, '$' | ',' | ';' | ' ' | '\t' | '\n' | 'p' | '=' | 'd');
        if !allowed {
            return Err(GrammarError::InvalidScript {
                command,
                script: script.to_string(),
            });
        }
    }
    Ok(())
}

fn parse_grep(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    if tokens.len() < 2 {
        return Err(GrammarError::MissingOperand {
            command,
            what: "pattern and file",
        });
    }
    let pattern = &tokens[0];
    reject_absolute_or_option_operand(pattern, command, false)?;
    argv.push(pattern.clone());
    for file in &tokens[1..] {
        reject_absolute_or_option_operand(file, command, false)?;
        path_operands.push(file.clone());
        argv.push(file.clone());
    }
    Ok(())
}

fn parse_plain_paths(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    if tokens.is_empty() {
        return Err(GrammarError::MissingOperand {
            command,
            what: "path",
        });
    }
    for path in tokens {
        reject_absolute_or_option_operand(path, command, false)?;
        path_operands.push(path.clone());
        argv.push(path.clone());
    }
    Ok(())
}

fn parse_head_tail(
    command: &'static str,
    tokens: &[String],
    argv: &mut Vec<String>,
    path_operands: &mut Vec<String>,
) -> Result<(), GrammarError> {
    let mut index = 0;
    if tokens.first().is_some_and(|token| token == "-n") {
        let value = tokens.get(1).ok_or(GrammarError::MissingOperand {
            command,
            what: "count after -n",
        })?;
        if !is_number(value) {
            return Err(GrammarError::NonNumeric {
                command,
                value: value.clone(),
            });
        }
        argv.push("-n".to_string());
        argv.push(value.clone());
        index = 2;
    }
    if index >= tokens.len() {
        return Err(GrammarError::MissingOperand {
            command,
            what: "path",
        });
    }
    for path in &tokens[index..] {
        reject_absolute_or_option_operand(path, command, false)?;
        path_operands.push(path.clone());
        argv.push(path.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(command: &str) -> ParsedCommand {
        parse_command(command).expect("grammar accepts command")
    }

    fn assert_rejected(command: &str) -> GrammarError {
        parse_command(command).expect_err("grammar must reject command")
    }

    #[test]
    fn git_status_and_short() {
        let parsed = parse("git status");
        assert_eq!(parsed.binary, Binary::Git);
        assert_eq!(parsed.argv, vec!["git", "status"]);
        assert!(parsed.path_operands.is_empty());

        let parsed = parse("git status --short");
        assert_eq!(parsed.argv, vec!["git", "status", "--short"]);
    }

    #[test]
    fn git_log_variants() {
        let parsed = parse("git log --oneline -n 5 -- src");
        assert_eq!(
            parsed.argv,
            vec!["git", "log", "--oneline", "-n", "5", "--", "src"]
        );
        assert_eq!(parsed.path_operands, vec!["src"]);

        let parsed = parse("git log");
        assert_eq!(parsed.argv, vec!["git", "log"]);
        assert!(parsed.path_operands.is_empty());
    }

    #[test]
    fn git_diff_and_show() {
        let parsed = parse("git diff --stat -- src/lib.rs");
        assert_eq!(
            parsed.argv,
            vec!["git", "diff", "--stat", "--", "src/lib.rs"]
        );
        assert_eq!(parsed.path_operands, vec!["src/lib.rs"]);

        let parsed = parse("git show HEAD -- src/lib.rs");
        assert_eq!(parsed.argv, vec!["git", "show", "HEAD", "--", "src/lib.rs"]);
        assert_eq!(parsed.path_operands, vec!["src/lib.rs"]);

        let parsed = parse("git show");
        assert_eq!(parsed.argv, vec!["git", "show"]);
        assert!(parsed.path_operands.is_empty());
    }

    #[test]
    fn rg_patterns_and_paths() {
        let parsed = parse("rg -n -i -- 'fn main' src");
        assert_eq!(parsed.argv, vec!["rg", "-n", "-i", "--", "fn main", "src"]);
        assert_eq!(parsed.path_operands, vec!["src"]);

        let parsed = parse("rg --no-heading -g '*.rs' -- foo src tests");
        assert_eq!(
            parsed.argv,
            vec![
                "rg",
                "--no-heading",
                "-g",
                "*.rs",
                "--",
                "foo",
                "src",
                "tests"
            ]
        );
        assert_eq!(parsed.path_operands, vec!["src", "tests"]);
    }

    #[test]
    fn find_predicates() {
        let parsed = parse("find . -name '*.rs' -type f -maxdepth 3 -print");
        assert_eq!(
            parsed.argv,
            vec![
                "find",
                ".",
                "-name",
                "*.rs",
                "-type",
                "f",
                "-maxdepth",
                "3",
                "-print"
            ]
        );
        assert_eq!(parsed.path_operands, vec!["."]);
    }

    #[test]
    fn sed_and_grep_and_coreutils() {
        let parsed = parse("sed -n '1,5p' README.md");
        assert_eq!(parsed.argv, vec!["sed", "-n", "1,5p", "README.md"]);
        assert_eq!(parsed.path_operands, vec!["README.md"]);

        let parsed = parse("grep 'needle' a.txt b.txt");
        assert_eq!(parsed.argv, vec!["grep", "needle", "a.txt", "b.txt"]);
        assert_eq!(parsed.path_operands, vec!["a.txt", "b.txt"]);

        let parsed = parse("cat a.txt b.txt");
        assert_eq!(parsed.argv, vec!["cat", "a.txt", "b.txt"]);

        let parsed = parse("head -n 3 a.txt");
        assert_eq!(parsed.argv, vec!["head", "-n", "3", "a.txt"]);

        let parsed = parse("tail a.txt");
        assert_eq!(parsed.argv, vec!["tail", "a.txt"]);

        let parsed = parse("ls . src");
        assert_eq!(parsed.argv, vec!["ls", ".", "src"]);

        let parsed = parse("wc a.txt");
        assert_eq!(parsed.argv, vec!["wc", "a.txt"]);
    }

    #[test]
    fn rejects_option_smuggling_operands() {
        assert_rejected("cat --help");
        assert_rejected("grep -R needle file");
        assert_rejected("git -C ~/.ssh log");
        assert_rejected("git --git-dir=/x status");
        assert_rejected("git diff --output=x");
        assert_rejected("git -c core.fsmonitor=x status");
        assert_rejected("find . -exec sh -c 'evil' \\;");
        assert_rejected("find . -execdir sh -c 'evil' \\;");
        assert_rejected("find . -ok rm {} \\;");
        assert_rejected("find . -delete");
        assert_rejected("find . -fprint x");
        assert_rejected("find -L .");
        assert_rejected("sed -n -i '1p' f");
        assert_rejected("sed -n --in-place '1p' f");
        assert_rejected("rg --pre 'sh -c evil' -- p f");
        assert_rejected("rg --pre-glob '*.sh' -- p f");
        assert_rejected("rg --follow -- p f");
        assert_rejected("rg -L -- p f");
        assert_rejected("git log --all");
        assert_rejected("git status --porcelain");
    }

    #[test]
    fn rejects_absolute_paths_and_redirection() {
        assert_rejected("cat /etc/passwd");
        assert_rejected("sed -n '1p' /etc/passwd");
        assert_rejected("grep x /etc/passwd");
        assert_rejected("ls /");
        assert_rejected("cat ../etc/passwd");
        assert_rejected("sed -n '1p' ../../etc/passwd");
        assert_rejected("grep x ../../outside");
        assert_rejected("echo x > out.txt");
        assert_rejected("cat a.txt > b.txt");
        assert_rejected("cat a.txt | grep x");
        assert_rejected("cat $(echo a)");
        assert_rejected("cat `echo a`");
        assert_rejected("cat a.txt; rm b.txt");
        assert_rejected("cat a.txt &");
    }

    #[test]
    fn rejects_bad_sed_scripts_and_numbers() {
        assert_rejected("sed -n '1e whoami' f");
        assert_rejected("sed -n '1w /tmp/x' f");
        assert_rejected("sed -n '1r /tmp/x' f");
        assert_rejected("sed -n '/x/e whoami' f");
        assert_rejected("sed '1p' f");
        assert_rejected("head -n x f");
        assert_rejected("git log -n abc");
        assert_rejected("find . -maxdepth x");
    }

    #[test]
    fn rejects_unknown_commands_and_malformed() {
        assert_rejected("");
        assert_rejected("sh -c whoami");
        assert_rejected("bash -c whoami");
        assert_rejected("curl http://evil");
        assert_rejected("awk '{print}'");
        assert_rejected("python -c 'evil'");
    }

    #[test]
    fn rejects_rg_without_separator_and_grep_without_file() {
        assert_rejected("rg -n pattern src");
        assert_rejected("rg -- pattern");
        assert_rejected("grep pattern");
    }

    #[test]
    fn accepts_literal_dash_pattern_after_rg_separator() {
        let parsed = parse("rg -- --flag src");
        assert_eq!(parsed.argv, vec!["rg", "--", "--flag", "src"]);
    }
}

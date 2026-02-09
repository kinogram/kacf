use anyhow::{anyhow, Context, Result};
use wait_timeout::ChildExt;
use std::time::Duration;
use regex::Regex;
use std::path::Path;
use std::process::{Command, Stdio};

/// Result of evaluating a command. Contains exit code, stdout, stderr.
pub struct EvalResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run the given evaluation command in the provided workspace directory.
///
/// The `cmdline` string is split into program and arguments. Returns
/// `EvalResult` containing exit status and captured standard output and error.
pub fn run_eval(workspace: &Path, cmdline: &str) -> Result<EvalResult> {
    // 拦截危险命令关键字，防止误删系统文件或其他破坏行为。
    let lowered = cmdline.to_lowercase();
    let dangerous = [
        "rm -rf", "del ", "format", "shutdown", "mkfs", "wipe", "rd ", "powershell remove-item", "sudo",
    ];
    for bad in &dangerous {
        if lowered.contains(bad) {
            return Err(anyhow!(format!("Dangerous command detected: {}", bad)));
        }
    }

    // 解析命令行
    let parts = shell_split(cmdline)?;
    if parts.is_empty() {
        return Err(anyhow!("Empty eval command"));
    }

    // 构建子进程
    let mut child_cmd = Command::new(&parts[0]);
    if parts.len() > 1 {
        child_cmd.args(&parts[1..]);
    }
    child_cmd.current_dir(workspace);
    child_cmd.stdout(Stdio::piped());
    child_cmd.stderr(Stdio::piped());

    // 启动进程
    let mut child = child_cmd.spawn().with_context(|| format!("run {}", cmdline))?;
    // 等待最长 120 秒，超时则杀死进程
    let timeout = Duration::from_secs(120);
    match child.wait_timeout(timeout).with_context(|| "wait_timeout failed")? {
        Some(status) => {
            // 进程在限定时间内结束
            let exit_code = status.code().unwrap_or(-1);
            // 读取 stdout/stderr
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(mut out) = child.stdout.take() {
                use std::io::Read;
                let mut buf = Vec::new();
                out.read_to_end(&mut buf)?;
                stdout = String::from_utf8_lossy(&buf).to_string();
            }
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                let mut buf = Vec::new();
                err.read_to_end(&mut buf)?;
                stderr = String::from_utf8_lossy(&buf).to_string();
            }
            Ok(EvalResult { exit_code, stdout, stderr })
        }
        None => {
            // 超时，强制终止
            let _ = child.kill();
            let _ = child.wait();
            Ok(EvalResult {
                exit_code: -2,
                stdout: String::new(),
                stderr: format!("Timed out after {:?}", timeout),
            })
        }
    }
}

/// Evaluate a regex pattern against the given text. Returns true if the pattern
/// matches anywhere in the text. Invalid regex patterns return false.
pub fn regex_match(pattern: &str, text: &str) -> bool {
    if pattern.trim().is_empty() {
        return true;
    }
    Regex::new(pattern)
        .ok()
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

/// Very simple shell-like splitting for a command string. Supports quoted
/// substrings but does not implement full shell semantics. Used to parse
/// evaluation commands from the UI.
fn shell_split(s: &str) -> Result<Vec<String>> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_quote = false;
    let mut quote_char = '\0';
    for ch in s.chars() {
        match ch {
            '"' | '\'' => {
                if in_quote && ch == quote_char {
                    in_quote = false;
                } else if !in_quote {
                    in_quote = true;
                    quote_char = ch;
                } else {
                    cur.push(ch);
                }
            }
            ' ' | '\t' if !in_quote => {
                if !cur.is_empty() {
                    out.push(cur.clone());
                    cur.clear();
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}
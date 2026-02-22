use std::path::Path;
use std::process::Command;

const MAX_SECTION: usize = 1200;
const MAX_PACKET: usize = 3800;

pub(crate) fn build_debug_packet(
    workspace: &Path,
    category: &str,
    signature: &str,
    key_lines: &[String],
    eval_stdout: &str,
    eval_stderr: &str,
) -> String {
    let ws = workspace.display().to_string();
    let git_status = run_in_workspace(workspace, &["status", "--short"]);
    let git_last = run_in_workspace(workspace, &["log", "--oneline", "-n", "3"]);
    let git_diff_stat = run_in_workspace(workspace, &["diff", "--stat"]);
    let rust_targets = list_key_files(workspace);

    let mut packet = String::new();
    packet.push_str("[Self-Debug Snapshot]\n");
    packet.push_str(&format!("workspace: {}\n", ws));
    packet.push_str(&format!("failure_category: {}\n", category));
    packet.push_str(&format!(
        "failure_signature: {}\n",
        truncate(signature, 220)
    ));
    if !key_lines.is_empty() {
        packet.push_str("failure_key_lines:\n");
        for line in key_lines.iter().take(8) {
            packet.push_str("  - ");
            packet.push_str(&truncate(line, 220));
            packet.push('\n');
        }
    }
    append_section(&mut packet, "git_status_short", &git_status);
    append_section(&mut packet, "git_last_3_commits", &git_last);
    append_section(&mut packet, "git_diff_stat", &git_diff_stat);
    append_section(&mut packet, "workspace_key_files", &rust_targets);
    append_section(
        &mut packet,
        "eval_stderr_tail",
        &tail_lines(eval_stderr, 40),
    );
    append_section(
        &mut packet,
        "eval_stdout_tail",
        &tail_lines(eval_stdout, 30),
    );
    truncate(&packet, MAX_PACKET)
}

fn append_section(out: &mut String, title: &str, body: &str) {
    out.push_str(&format!("{title}:\n"));
    if body.trim().is_empty() {
        out.push_str("  (empty)\n");
    } else {
        out.push_str(&indent_block(&truncate(body, MAX_SECTION), "  "));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
}

fn run_in_workspace(workspace: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output();
    match output {
        Ok(o) => {
            let mut s = String::new();
            if !o.stdout.is_empty() {
                s.push_str(&String::from_utf8_lossy(&o.stdout));
            }
            if !o.stderr.is_empty() {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            if s.trim().is_empty() && !o.status.success() {
                format!("git {:?} failed with {}", args, o.status)
            } else {
                s
            }
        }
        Err(e) => format!("git {:?} spawn failed: {}", args, e),
    }
}

fn list_key_files(workspace: &Path) -> String {
    let output = Command::new("sh")
        .args([
            "-lc",
            "find . -maxdepth 3 -type f | sed 's#^\\./##' | head -n 60",
        ])
        .current_dir(workspace)
        .output();
    match output {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            if text.trim().is_empty() {
                "(none)".to_string()
            } else {
                text
            }
        }
        Err(e) => format!("find key files failed: {}", e),
    }
}

fn tail_lines(input: &str, n: usize) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

fn indent_block(text: &str, prefix: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(prefix);
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::from(&s[..end]);
    out.push_str(" ...[truncated]");
    out
}

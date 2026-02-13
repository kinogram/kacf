#[derive(Debug, Clone)]
pub(crate) struct FailureDigest {
    pub(crate) category: &'static str,
    pub(crate) signature: String,
    pub(crate) key_lines: Vec<String>,
}

pub(crate) fn digest_eval_failure(exit_code: i32, stdout: &str, stderr: &str) -> FailureDigest {
    let joined = format!("{stdout}\n{stderr}");
    let lowered = joined.to_lowercase();
    let category =
        if exit_code == -2 || lowered.contains("timed out") || lowered.contains("timeout") {
            "timeout"
        } else if lowered.contains("error[") || lowered.contains("could not compile") {
            "compile_error"
        } else if lowered.contains("test result: failed")
            || lowered.contains("failures:")
            || lowered.contains("assertion failed")
        {
            "test_failure"
        } else if lowered.contains("panicked") || lowered.contains("traceback") {
            "runtime_crash"
        } else {
            "runtime_or_logic_failure"
        };

    let key_lines = collect_key_lines(stdout, stderr, 8);
    let signature = key_lines
        .first()
        .cloned()
        .unwrap_or_else(|| format!("exit_code={exit_code}"));

    FailureDigest {
        category,
        signature,
        key_lines,
    }
}

pub(crate) fn eval_has_fatal_runtime_marker(stdout: &str, stderr: &str) -> bool {
    let lowered = format!("{stdout}\n{stderr}").to_lowercase();
    lowered.contains("thread 'main' panicked")
        || lowered.contains(" panicked at ")
        || lowered.contains("xopendisplay() failed")
        || lowered.contains("segmentation fault")
        || lowered.contains("stack backtrace:")
        || lowered.contains("fatal runtime error")
}

pub(crate) fn failure_severity(category: &str) -> u8 {
    match category {
        "compile_error" => 3,
        "runtime_crash" => 3,
        "test_failure" => 2,
        "runtime_or_logic_failure" => 2,
        "timeout" => 1,
        _ => 1,
    }
}

pub(crate) fn category_changed_worse(previous: &str, current: &str) -> bool {
    failure_severity(current) > failure_severity(previous)
}

fn collect_key_lines(stdout: &str, stderr: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        maybe_add_key_line(&mut out, line, limit);
        if out.len() >= limit {
            return out;
        }
    }
    for line in stdout.lines() {
        maybe_add_key_line(&mut out, line, limit);
        if out.len() >= limit {
            return out;
        }
    }
    out
}

fn maybe_add_key_line(out: &mut Vec<String>, line: &str, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let l = line.trim();
    if l.is_empty() || l.len() > 220 {
        return;
    }
    let lowered = l.to_lowercase();
    let hit = lowered.contains("error")
        || lowered.contains("failed")
        || lowered.contains("panic")
        || lowered.contains("assert")
        || lowered.contains("exception")
        || lowered.contains("traceback")
        || lowered.contains("timeout")
        || lowered.contains("could not compile")
        || lowered.contains("caused by");
    if hit && !out.iter().any(|x| x == l) {
        out.push(l.to_string());
    }
}

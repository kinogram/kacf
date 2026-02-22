#[derive(Debug, Clone)]
pub(crate) struct FailureDigest {
    pub(crate) category: &'static str,
    pub(crate) signature: String,
    pub(crate) key_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RepairStrategy {
    pub(crate) root_cause_focus: &'static str,
    pub(crate) patch_scope: &'static str,
    pub(crate) test_focus: &'static str,
    pub(crate) extra_checks: &'static str,
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

pub(crate) fn repair_strategy_for(
    category: &str,
    consecutive_eval_failures: u32,
    consecutive_same_failure: u32,
) -> RepairStrategy {
    let mut strategy = match category {
        "compile_error" => RepairStrategy {
            root_cause_focus:
                "先定位首个编译错误（通常是类型/导入/签名不一致），不要同时修多个模块。",
            patch_scope: "优先改动直接报错文件与其最小依赖，避免重构。",
            test_focus: "先确保可编译，再恢复测试通过。",
            extra_checks: "检查 Cargo.toml、feature flag、模块路径与公开可见性。",
        },
        "runtime_crash" => RepairStrategy {
            root_cause_focus: "优先修复 panic/崩溃根因，先保程序可运行。",
            patch_scope: "只改崩溃路径与必要防御逻辑（边界检查/空值处理）。",
            test_focus: "补充能稳定复现崩溃的最小测试，防止回归。",
            extra_checks: "检查初始化顺序、资源句柄、并发共享状态。",
        },
        "test_failure" => RepairStrategy {
            root_cause_focus: "先确认失败测试的预期与实现是否一致，再最小修复。",
            patch_scope: "优先修改业务逻辑或断言之一，不要同时大改两者。",
            test_focus: "为修复点补充针对性测试，避免仅修表面现象。",
            extra_checks: "排查时间/随机性/顺序依赖导致的不稳定测试。",
        },
        "timeout" => RepairStrategy {
            root_cause_focus: "定位卡住阶段（构建、启动、测试等待），修复阻塞点。",
            patch_scope: "最小化调整循环/等待/IO，必要时加入超时保护。",
            test_focus: "增加快速失败路径，确保脚本能在预期时间结束。",
            extra_checks: "检查死循环、等待条件、端口占用与外部依赖可用性。",
        },
        _ => RepairStrategy {
            root_cause_focus: "先给出可验证根因，再最小修复。",
            patch_scope: "控制改动面，先恢复主路径正确性。",
            test_focus: "补充能复现问题的检查，再验证修复。",
            extra_checks: "检查日志中首个错误与后续连锁错误的因果关系。",
        },
    };

    if consecutive_same_failure >= 2 {
        strategy.patch_scope = "同签名重复失败：必须缩小为单一根因修复，不允许大范围重写。";
    }
    if consecutive_eval_failures >= 4 {
        strategy.extra_checks = "连续失败较多：额外核对评测脚本、入口参数、环境依赖与缓存状态。";
    }
    strategy
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

pub(crate) fn build_repair_prompt(
    eval_cmd: &str,
    iter: u32,
    result: &crate::runner::EvalResult,
    digest: &crate::protocol_failure::FailureDigest,
    consecutive_eval_failures: u32,
    consecutive_same_failure: u32,
    diff_for_feedback: &str,
) -> String {
    let repeated_hint = if consecutive_same_failure >= 2 {
        "检测到同类错误重复出现。你必须先写出根因，再给出最小修复，并补充/修正测试以防回归。"
    } else {
        "请基于错误日志做最小必要修复。"
    };
    let strictness_hint = if consecutive_eval_failures >= 3 {
        "你需要额外检查：依赖版本、构建脚本、入口参数、测试脚本本身是否错误。"
    } else {
        ""
    };
    let key_lines_text = if digest.key_lines.is_empty() {
        "（无）".to_string()
    } else {
        digest
            .key_lines
            .iter()
            .map(|x| format!("- {x}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "本地评测失败，请继续修复。\n\
迭代轮次: {iter}\n\
命令: {cmd}\n\
exit_code: {exit}\n\
错误类别: {category}\n\
失败签名: {signature}\n\
连续失败次数: {fail_count}\n\
同签名重复次数: {same_count}\n\
\n\
关键错误行:\n{key_lines}\n\
\n\
最近补丁 diff 摘要:\n{diff}\n\
\n\
stdout:\n{stdout}\n\
\n\
stderr:\n{stderr}\n\
\n\
修复要求:\n\
1) {repeated}\n\
2) 先修复导致失败的直接原因，再处理次要问题。\n\
3) 如果评测脚本本身不准确，先修正评测脚本再修代码。\n\
4) 输出必须是 kind=patch JSON，且只输出 JSON。\n\
5) 小步修改，避免大面积重写。\n\
6) 修复后必须确保 `{cmd}` 可通过。{strictness}",
        cmd = eval_cmd,
        exit = result.exit_code,
        category = digest.category,
        signature = digest.signature,
        fail_count = consecutive_eval_failures,
        same_count = consecutive_same_failure,
        key_lines = key_lines_text,
        diff = if diff_for_feedback.is_empty() {
            "（当前 diff 不可用）"
        } else {
            diff_for_feedback
        },
        stdout = crate::protocol_patch::truncate(&result.stdout, 8000),
        stderr = crate::protocol_patch::truncate(&result.stderr, 8000),
        repeated = repeated_hint,
        strictness = strictness_hint,
    )
}

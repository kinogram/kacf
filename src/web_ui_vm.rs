use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wait_timeout::ChildExt;

use crate::auth::AccountRole;
use crate::lock_utils::lock_recover;
use crate::web_ui::AppState;
use crate::web_ui_authz;
use crate::web_ui_models::{
    VmActionPayload, VmActionResponse, VmBootstrapPayload, VmCapability, VmClonePayload,
    VmDeletePayload, VmExecBatchTaskPayload, VmExecCancelPayload, VmExecCancelResponse,
    VmExecCustomProfileDeletePayload, VmExecCustomProfileSavePayload, VmExecEnqueueBatchPayload,
    VmExecDispatchResponse, VmExecDispatchTraceCandidate, VmExecDispatchTraceEntry,
    VmExecDispatchTraceQuery, VmExecDispatchTraceResponse, VmExecEnqueuePayload,
    VmExecEnqueueProfilePayload, VmExecPayload, VmExecProfileDetailQuery,
    VmExecProfileDetailResponse, VmExecProfilePreviewQuery, VmExecProfilePreviewResponse,
    VmExecProfilesResponse, VmExecQueueItem, VmExecResponse, VmInstance, VmLogQuery,
    VmLogsResponse, VmHealthAction, VmHealthIssue, VmHealthScanPayload, VmHealthScanResponse,
    VmPolicyConfig, VmPolicyRoleLimits, VmPolicyScheduler,
    VmProvisionPayload, VmQueueCancelPayload, VmQueueQuery, VmQueueResponse,
    VmQueueStatsResponse, VmReadyQuery, VmReadyResponse, VmSelfDebugPlanPayload,
    VmSelfDebugPlanResponse, VmSelfDebugRunDetailQuery, VmSelfDebugRunDetailResponse,
    VmSelfDebugContextResponse, VmSelfDebugHistoryArchivePayload,
    VmSelfDebugHistoryArchiveResponse, VmSelfDebugHistoryClearPayload,
    VmSelfDebugHistoryClearResponse, VmSelfDebugHistoryDetailResponse, VmSelfDebugHistoryEntry,
    VmSelfDebugHistoryResponse,
    VmSelfDebugRunSummary, VmSelfDebugRunTaskDetail,
    VmSelfDebugRunsQuery, VmSelfDebugRunsResponse, VmSelfDebugStopPayload,
    VmSelfDebugStrategyRulesResponse, VmSelfDebugStrategyRulesSavePayload, VmSelfDebugStrategyStat,
    VmSelfDebugStrategyStatsResponse, VmSnapshotEntry,
    VmSnapshotListQuery, VmSnapshotListResponse, VmSnapshotPayload, VmStateStore, VmStatusResponse,
};

const VM_DIR: &str = "vm";
const VM_DISK_DIR: &str = "disks";
const VM_RUNTIME_DIR: &str = "runtime";
const VM_LOG_DIR: &str = "logs";
const VM_STATE_FILE: &str = "vm_state.json";
const VM_POLICY_FILE: &str = "vm_policy.json";
const VM_PROFILES_FILE: &str = "vm_exec_profiles.json";
const VM_STRATEGY_RULES_FILE: &str = "vm_self_debug_strategy_rules.json";
const VM_HISTORY_SUFFIX: &str = ".self_debug.history.json";
const VM_HISTORY_MAX_ENTRIES: usize = 200;
const VM_DISPATCH_TRACE_FILE: &str = "vm_exec_dispatch.trace.json";
const VM_DISPATCH_TRACE_MAX_ENTRIES: usize = 120;
const VM_MAX_INSTANCES_USER: usize = 4;
const VM_MAX_INSTANCES_ADMIN: usize = 16;
const VM_QUEUE_MAX_ITEMS_USER: usize = 400;
const VM_QUEUE_MAX_ITEMS_ADMIN: usize = 2000;
const VM_RUNNING_MAX_USER: usize = 2;
const VM_RUNNING_MAX_ADMIN: usize = 8;
const VM_EXEC_RUNNING_MAX_USER: usize = 4;
const VM_EXEC_RUNNING_MAX_ADMIN: usize = 16;
const VM_EXEC_HARD_TIMEOUT_GRACE_SEC: u64 = 15;
const VM_PRIORITY_AGING_STEP_SEC: u64 = 60;
const VM_PRIORITY_AGING_MAX_BOOST: i32 = 20;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn vm_root_path(managed_root_dir: &str) -> PathBuf {
    workspace_root().join(managed_root_dir).join(VM_DIR)
}

fn vm_state_path(managed_root_dir: &str) -> PathBuf {
    vm_root_path(managed_root_dir).join(VM_STATE_FILE)
}

fn vm_policy_path(managed_root_dir: &str) -> PathBuf {
    vm_root_path(managed_root_dir).join(VM_POLICY_FILE)
}

fn vm_disk_dir(managed_root_dir: &str) -> PathBuf {
    vm_root_path(managed_root_dir).join(VM_DISK_DIR)
}

fn vm_runtime_dir(managed_root_dir: &str) -> PathBuf {
    vm_root_path(managed_root_dir).join(VM_RUNTIME_DIR)
}

fn vm_log_dir(managed_root_dir: &str) -> PathBuf {
    vm_root_path(managed_root_dir).join(VM_LOG_DIR)
}

fn vm_profiles_path(managed_root_dir: &str) -> PathBuf {
    vm_root_path(managed_root_dir).join(VM_PROFILES_FILE)
}

fn vm_strategy_rules_path(managed_root_dir: &str) -> PathBuf {
    vm_root_path(managed_root_dir).join(VM_STRATEGY_RULES_FILE)
}

fn vm_dispatch_trace_path(managed_root_dir: &str) -> PathBuf {
    vm_runtime_dir(managed_root_dir).join(VM_DISPATCH_TRACE_FILE)
}

fn vm_pid_path(managed_root_dir: &str, vm_name: &str) -> PathBuf {
    vm_runtime_dir(managed_root_dir).join(format!("{vm_name}.pid"))
}

fn vm_log_path(managed_root_dir: &str, vm_name: &str) -> PathBuf {
    vm_log_dir(managed_root_dir).join(format!("{vm_name}.log"))
}

fn vm_exec_pid_path(managed_root_dir: &str, vm_name: &str) -> PathBuf {
    vm_runtime_dir(managed_root_dir).join(format!("{vm_name}.exec.pid"))
}

fn vm_exec_cancel_path(managed_root_dir: &str, vm_name: &str) -> PathBuf {
    vm_runtime_dir(managed_root_dir).join(format!("{vm_name}.exec.cancel"))
}

fn vm_exec_queue_path(managed_root_dir: &str, vm_name: &str) -> PathBuf {
    vm_runtime_dir(managed_root_dir).join(format!("{vm_name}.exec.queue.json"))
}

fn vm_exec_worker_lock_path(managed_root_dir: &str, vm_name: &str) -> PathBuf {
    vm_runtime_dir(managed_root_dir).join(format!("{vm_name}.exec.worker.lock"))
}

fn vm_self_debug_history_path(managed_root_dir: &str, vm_name: &str) -> PathBuf {
    vm_runtime_dir(managed_root_dir).join(format!("{vm_name}{VM_HISTORY_SUFFIX}"))
}

fn default_vm_policy_config() -> VmPolicyConfig {
    VmPolicyConfig {
        user: VmPolicyRoleLimits {
            vm_instance_max: VM_MAX_INSTANCES_USER,
            vm_queue_max_items: VM_QUEUE_MAX_ITEMS_USER,
            vm_running_max: VM_RUNNING_MAX_USER,
            vm_exec_running_max: VM_EXEC_RUNNING_MAX_USER,
        },
        admin: VmPolicyRoleLimits {
            vm_instance_max: VM_MAX_INSTANCES_ADMIN,
            vm_queue_max_items: VM_QUEUE_MAX_ITEMS_ADMIN,
            vm_running_max: VM_RUNNING_MAX_ADMIN,
            vm_exec_running_max: VM_EXEC_RUNNING_MAX_ADMIN,
        },
        scheduler: VmPolicyScheduler {
            exec_hard_timeout_grace_sec: VM_EXEC_HARD_TIMEOUT_GRACE_SEC,
            priority_aging_step_sec: VM_PRIORITY_AGING_STEP_SEC,
            priority_aging_max_boost: VM_PRIORITY_AGING_MAX_BOOST,
        },
    }
}

fn sanitize_vm_policy_config(mut cfg: VmPolicyConfig) -> VmPolicyConfig {
    cfg.user.vm_instance_max = cfg.user.vm_instance_max.clamp(1, 64);
    cfg.user.vm_queue_max_items = cfg.user.vm_queue_max_items.clamp(1, 20_000);
    cfg.user.vm_running_max = cfg.user.vm_running_max.clamp(1, 32);
    cfg.user.vm_exec_running_max = cfg.user.vm_exec_running_max.clamp(1, 64);

    cfg.admin.vm_instance_max = cfg.admin.vm_instance_max.clamp(1, 256);
    cfg.admin.vm_queue_max_items = cfg.admin.vm_queue_max_items.clamp(1, 100_000);
    cfg.admin.vm_running_max = cfg.admin.vm_running_max.clamp(1, 128);
    cfg.admin.vm_exec_running_max = cfg.admin.vm_exec_running_max.clamp(1, 256);

    if cfg.admin.vm_instance_max < cfg.user.vm_instance_max {
        cfg.admin.vm_instance_max = cfg.user.vm_instance_max;
    }
    if cfg.admin.vm_queue_max_items < cfg.user.vm_queue_max_items {
        cfg.admin.vm_queue_max_items = cfg.user.vm_queue_max_items;
    }
    if cfg.admin.vm_running_max < cfg.user.vm_running_max {
        cfg.admin.vm_running_max = cfg.user.vm_running_max;
    }
    if cfg.admin.vm_exec_running_max < cfg.user.vm_exec_running_max {
        cfg.admin.vm_exec_running_max = cfg.user.vm_exec_running_max;
    }

    cfg.scheduler.exec_hard_timeout_grace_sec =
        cfg.scheduler.exec_hard_timeout_grace_sec.clamp(0, 600);
    cfg.scheduler.priority_aging_step_sec = cfg.scheduler.priority_aging_step_sec.clamp(1, 3600);
    cfg.scheduler.priority_aging_max_boost = cfg.scheduler.priority_aging_max_boost.clamp(0, 100);
    cfg
}

fn load_vm_policy_config(managed_root_dir: &str) -> VmPolicyConfig {
    let path = vm_policy_path(managed_root_dir);
    let raw = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return default_vm_policy_config(),
    };
    let parsed = serde_json::from_str::<VmPolicyConfig>(&raw).unwrap_or_else(|_| default_vm_policy_config());
    sanitize_vm_policy_config(parsed)
}

fn save_vm_policy_config(managed_root_dir: &str, cfg: &VmPolicyConfig) -> std::io::Result<()> {
    let path = vm_policy_path(managed_root_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&sanitize_vm_policy_config(cfg.clone()))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn vm_role_limits<'a>(cfg: &'a VmPolicyConfig, role: &AccountRole) -> &'a VmPolicyRoleLimits {
    match role {
        AccountRole::Admin => &cfg.admin,
        AccountRole::User | AccountRole::Guest => &cfg.user,
    }
}

fn vm_instance_limit_by_role(managed_root_dir: &str, role: &AccountRole) -> usize {
    vm_role_limits(&load_vm_policy_config(managed_root_dir), role).vm_instance_max
}

fn vm_queue_limit_by_role(managed_root_dir: &str, role: &AccountRole) -> usize {
    vm_role_limits(&load_vm_policy_config(managed_root_dir), role).vm_queue_max_items
}

fn vm_running_limit_by_role(managed_root_dir: &str, role: &AccountRole) -> usize {
    vm_role_limits(&load_vm_policy_config(managed_root_dir), role).vm_running_max
}

fn vm_exec_running_limit_by_role(managed_root_dir: &str, role: &AccountRole) -> usize {
    vm_role_limits(&load_vm_policy_config(managed_root_dir), role).vm_exec_running_max
}

fn ensure_queue_capacity(existing: usize, adding: usize, limit: usize) -> Result<(), String> {
    if adding == 0 {
        return Ok(());
    }
    let next = existing.saturating_add(adding);
    if next > limit {
        return Err(format!(
            "vm exec queue limit exceeded: current={} adding={} limit={}",
            existing, adding, limit
        ));
    }
    Ok(())
}

fn append_vm_log(managed_root_dir: &str, vm_name: &str, line: &str) {
    let dir = vm_log_dir(managed_root_dir);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = vm_log_path(managed_root_dir, vm_name);
    let stamped = format!("[{}] {}\n", now_unix(), line);
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, stamped.as_bytes()));
}

fn tail_text(raw: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let text = raw.to_string();
    let total = text.chars().count();
    if total <= max_chars {
        return text;
    }
    text.chars().skip(total - max_chars).collect()
}

fn build_exec_output_preview(stdout: &str, stderr: &str, max_chars: usize) -> String {
    let out = tail_text(stdout, max_chars / 2);
    let err = tail_text(stderr, max_chars / 2);
    if out.is_empty() && err.is_empty() {
        return String::new();
    }
    format!("stdout:\n{}\n\nstderr:\n{}", out, err)
}

fn load_exec_queue(managed_root_dir: &str, vm_name: &str) -> Vec<VmExecQueueItem> {
    let path = vm_exec_queue_path(managed_root_dir, vm_name);
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<VmExecQueueItem>>(&text).unwrap_or_default()
}

fn save_exec_queue(
    managed_root_dir: &str,
    vm_name: &str,
    items: &[VmExecQueueItem],
) -> std::io::Result<()> {
    let path = vm_exec_queue_path(managed_root_dir, vm_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(items)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn load_dispatch_trace(managed_root_dir: &str) -> Vec<VmExecDispatchTraceEntry> {
    let path = vm_dispatch_trace_path(managed_root_dir);
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<VmExecDispatchTraceEntry>>(&text).unwrap_or_default()
}

fn save_dispatch_trace(
    managed_root_dir: &str,
    entries: &[VmExecDispatchTraceEntry],
) -> std::io::Result<()> {
    let path = vm_dispatch_trace_path(managed_root_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(entries)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn append_dispatch_trace_entry(managed_root_dir: &str, entry: VmExecDispatchTraceEntry) {
    let mut entries = load_dispatch_trace(managed_root_dir);
    entries.push(entry);
    if entries.len() > VM_DISPATCH_TRACE_MAX_ENTRIES {
        let drop_count = entries.len().saturating_sub(VM_DISPATCH_TRACE_MAX_ENTRIES);
        entries.drain(0..drop_count);
    }
    let _ = save_dispatch_trace(managed_root_dir, &entries);
}

fn count_running_exec_tasks_all_vms(managed_root_dir: &str, state: &VmStateStore) -> usize {
    state
        .vms
        .keys()
        .map(|vm_name| {
            load_exec_queue(managed_root_dir, vm_name)
                .iter()
                .filter(|x| x.status == "running")
                .count()
        })
        .sum()
}

fn effective_task_timeout_sec(item: &VmExecQueueItem) -> u64 {
    if item.timeout_sec == 0 {
        30
    } else {
        item.timeout_sec.clamp(1, 3600)
    }
}

fn is_running_task_hard_timed_out(item: &VmExecQueueItem, now: u64, grace_sec: u64) -> bool {
    if item.status != "running" || item.started_at_unix == 0 {
        return false;
    }
    let timeout = effective_task_timeout_sec(item);
    let deadline = item
        .started_at_unix
        .saturating_add(timeout)
        .saturating_add(grace_sec);
    now > deadline
}

fn recover_stale_running_tasks_in_queue(
    items: &mut [VmExecQueueItem],
    now: u64,
    grace_sec: u64,
) -> usize {
    let mut recovered = 0usize;
    for item in items.iter_mut() {
        if !is_running_task_hard_timed_out(item, now, grace_sec) {
            continue;
        }
        let timeout = effective_task_timeout_sec(item);
        item.status = "failed".to_string();
        item.finished_at_unix = now;
        item.exit_code = -1;
        item.next_run_after_unix = 0;
        item.message = format!(
            "watchdog hard-timeout recovered task (timeout={}s+{}s)",
            timeout, grace_sec
        );
        recovered += 1;
    }
    recovered
}

fn queue_watchdog_recovery_stats(items: &[VmExecQueueItem]) -> (usize, u64) {
    let mut total = 0usize;
    let mut last = 0u64;
    for item in items {
        if !item
            .message
            .starts_with("watchdog hard-timeout recovered task")
        {
            continue;
        }
        total += 1;
        last = last.max(item.finished_at_unix);
    }
    (total, last)
}

fn force_stop_vm_exec_process(managed_root_dir: &str, vm_name: &str) {
    let pid_path = vm_exec_pid_path(managed_root_dir, vm_name);
    let cancel_path = vm_exec_cancel_path(managed_root_dir, vm_name);
    let _ = fs::write(&cancel_path, "cancel");
    if let Some(pid) = read_pid_file(&pid_path) {
        let _ = stop_pid(pid);
    }
    let _ = fs::remove_file(pid_path);
    let _ = fs::remove_file(cancel_path);
}

fn recover_stale_running_tasks_all_vms(
    managed_root_dir: &str,
    state: &VmStateStore,
    now: u64,
) -> usize {
    let mut total = 0usize;
    let grace_sec = load_vm_policy_config(managed_root_dir)
        .scheduler
        .exec_hard_timeout_grace_sec;
    for vm_name in state.vms.keys() {
        let mut items = load_exec_queue(managed_root_dir, vm_name);
        let recovered = recover_stale_running_tasks_in_queue(&mut items, now, grace_sec);
        if recovered == 0 {
            continue;
        }
        force_stop_vm_exec_process(managed_root_dir, vm_name);
        if let Err(e) = save_exec_queue(managed_root_dir, vm_name, &items) {
            append_vm_log(
                managed_root_dir,
                vm_name,
                &format!("exec watchdog save queue failed: {e}"),
            );
            continue;
        }
        append_vm_log(
            managed_root_dir,
            vm_name,
            &format!(
                "exec watchdog recovered stale running tasks={} grace_sec={}",
                recovered, grace_sec
            ),
        );
        total += recovered;
    }
    total
}

fn queue_task_id() -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("q-{}-{}", now_unix(), ns)
}

fn self_debug_run_id() -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("sd-{}-{}", now_unix(), ns)
}

fn sanitize_self_debug_run_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if value.len() > 64 {
        return None;
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(value.to_string())
}

fn retry_backoff_secs(retry_count: u32) -> u64 {
    let exp = retry_count.saturating_sub(1).min(6);
    let base = 5u64;
    base.saturating_mul(1u64 << exp).min(300)
}

fn queue_item_from_values(
    cmd: &str,
    timeout_sec: u64,
    wait_ready_sec: u64,
    priority: i32,
    retry_max: u32,
) -> VmExecQueueItem {
    let (risk_level, risk_tags) = classify_command_risk(cmd);
    VmExecQueueItem {
        id: queue_task_id(),
        command: cmd.to_string(),
        status: "pending".to_string(),
        created_at_unix: now_unix(),
        timeout_sec,
        wait_ready_sec,
        priority,
        retry_max,
        retry_count: 0,
        next_run_after_unix: 0,
        started_at_unix: 0,
        finished_at_unix: 0,
        exit_code: 0,
        message: String::new(),
        output_preview: String::new(),
        failure_category: String::new(),
        failure_signature: String::new(),
        failure_key_lines: Vec::new(),
        run_id: String::new(),
        run_kind: String::new(),
        run_max_runtime_sec: 0,
        command_risk_level: risk_level,
        command_risk_tags: risk_tags,
        strategy_signature: String::new(),
        trigger_task_id: String::new(),
    }
}

fn classify_command_risk(cmd: &str) -> (String, Vec<String>) {
    let text = cmd.trim();
    if text.is_empty() {
        return ("low".to_string(), Vec::new());
    }
    let low = text.to_ascii_lowercase();
    let mut tags: Vec<String> = Vec::new();
    let mut score: u8 = 0;

    let medium_hits = [
        ("sudo ", "privilege"),
        ("apt-get ", "pkg-manager"),
        ("dnf ", "pkg-manager"),
        ("yum ", "pkg-manager"),
        ("apk ", "pkg-manager"),
        ("pip install", "pkg-manager"),
        ("npm install", "pkg-manager"),
        ("cargo install", "pkg-manager"),
        ("chmod ", "permission"),
        ("chown ", "permission"),
        ("systemctl ", "service"),
    ];
    for (pat, tag) in medium_hits {
        if low.contains(pat) {
            score = score.max(1);
            if !tags.iter().any(|x| x == tag) {
                tags.push(tag.to_string());
            }
        }
    }

    let high_hits = [
        ("rm -rf /", "destructive"),
        ("mkfs", "filesystem"),
        ("dd if=", "disk-write"),
        ("shutdown", "system-power"),
        ("reboot", "system-power"),
        ("userdel ", "account"),
        ("groupdel ", "account"),
        ("iptables ", "network-firewall"),
        ("nft ", "network-firewall"),
        ("curl ", "network-download"),
        ("wget ", "network-download"),
    ];
    for (pat, tag) in high_hits {
        if low.contains(pat) {
            score = score.max(2);
            if !tags.iter().any(|x| x == tag) {
                tags.push(tag.to_string());
            }
        }
    }

    let level = match score {
        0 => "low",
        1 => "medium",
        _ => "high",
    };
    (level.to_string(), tags)
}

fn extract_failure_key_lines(stderr: &str, stdout: &str, message: &str) -> Vec<String> {
    let mut out = Vec::new();
    for src in [stderr, stdout, message] {
        for line in src.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let low = t.to_ascii_lowercase();
            let hit = low.contains("error")
                || low.contains("failed")
                || low.contains("panic")
                || low.contains("exception")
                || low.contains("timeout")
                || low.contains("not found")
                || low.contains("permission denied")
                || low.contains("assert");
            if hit {
                out.push(t.chars().take(220).collect::<String>());
                if out.len() >= 6 {
                    return out;
                }
            }
        }
    }
    if out.is_empty() && !message.trim().is_empty() {
        out.push(message.trim().chars().take(220).collect());
    }
    out
}

fn classify_failure(
    exit_code: i32,
    stderr: &str,
    stdout: &str,
    message: &str,
) -> (String, String, Vec<String>) {
    if exit_code == 0 || exit_code == 10 {
        return (String::new(), String::new(), Vec::new());
    }
    if exit_code == -2 {
        return ("canceled".to_string(), "execution canceled".to_string(), Vec::new());
    }
    let full = format!(
        "{}\n{}\n{}",
        stderr.to_ascii_lowercase(),
        stdout.to_ascii_lowercase(),
        message.to_ascii_lowercase()
    );
    let category = if full.contains("timeout") || exit_code == 124 || exit_code == 137 {
        "timeout"
    } else if full.contains("permission denied") {
        "permission"
    } else if full.contains("not found") || full.contains("no such file") || full.contains("command not found") {
        "missing_dependency"
    } else if full.contains("assert") || full.contains("test failed") || full.contains("failures:") {
        "test_failure"
    } else if full.contains("panic") || full.contains("exception") || full.contains("traceback") {
        "runtime_exception"
    } else if full.contains("compile") || full.contains("syntax error") || full.contains("cannot find") {
        "build_error"
    } else {
        "unknown_failure"
    };
    let key_lines = extract_failure_key_lines(stderr, stdout, message);
    let signature = if let Some(first) = key_lines.first() {
        first.clone()
    } else {
        message.trim().chars().take(160).collect()
    };
    (category.to_string(), signature, key_lines)
}

fn default_strategy_command_for_failure_category(category: &str) -> &'static str {
    match category {
        "missing_dependency" => "sh -lc 'if command -v apt-get >/dev/null 2>&1; then export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y build-essential git curl python3 python3-pip nodejs npm || true; elif command -v dnf >/dev/null 2>&1; then dnf install -y gcc gcc-c++ make git curl python3 python3-pip nodejs npm || true; elif command -v yum >/dev/null 2>&1; then yum install -y gcc gcc-c++ make git curl python3 python3-pip nodejs npm || true; elif command -v apk >/dev/null 2>&1; then apk add --no-cache build-base git curl python3 py3-pip nodejs npm || true; else echo no-supported-pkg-manager; fi'",
        "permission" => "sh -lc 'chmod -R u+rw . 2>/dev/null || true; find . -type d -exec chmod u+rwx {} + 2>/dev/null || true'",
        "timeout" => "sh -lc 'echo [self-debug][strategy] timeout-diagnostics; ps aux --sort=-%cpu | head -n 20; ps aux --sort=-%mem | head -n 20'",
        "build_error" => "sh -lc 'echo [self-debug][strategy] build-error-context; git status --short; git diff --stat'",
        "test_failure" => "sh -lc 'echo [self-debug][strategy] test-failure-context; git status --short; git diff --stat'",
        "runtime_exception" => "sh -lc 'echo [self-debug][strategy] runtime-exception-context; git status --short; git diff --stat'",
        _ => "sh -lc 'echo [self-debug][strategy] generic-failure-context; git status --short; git diff --stat'",
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct VmSelfDebugStrategyRulesStore {
    #[serde(default)]
    rules: BTreeMap<String, String>,
}

fn load_strategy_rules(managed_root_dir: &str) -> VmSelfDebugStrategyRulesStore {
    let path = vm_strategy_rules_path(managed_root_dir);
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return VmSelfDebugStrategyRulesStore::default(),
    };
    serde_json::from_str::<VmSelfDebugStrategyRulesStore>(&text).unwrap_or_default()
}

fn save_strategy_rules(
    managed_root_dir: &str,
    store: &VmSelfDebugStrategyRulesStore,
) -> std::io::Result<()> {
    let path = vm_strategy_rules_path(managed_root_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(store)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn sanitize_strategy_rules(raw: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let allowed = [
        "missing_dependency",
        "permission",
        "timeout",
        "build_error",
        "test_failure",
        "runtime_exception",
        "unknown_failure",
    ];
    let mut out = BTreeMap::new();
    for key in allowed {
        let Some(cmd) = raw.get(key) else {
            continue;
        };
        let t = cmd.trim();
        if t.is_empty() || t.len() > 2000 {
            continue;
        }
        out.insert(key.to_string(), t.to_string());
    }
    out
}

fn strategy_command_for_failure_category(managed_root_dir: &str, category: &str) -> String {
    let store = load_strategy_rules(managed_root_dir);
    if let Some(cmd) = store.rules.get(category) {
        let t = cmd.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    default_strategy_command_for_failure_category(category).to_string()
}

fn strategy_injected_count(items: &[VmExecQueueItem], run_id: &str, signature: &str) -> usize {
    items.iter()
        .filter(|x| {
            x.run_id == run_id
                && x.run_kind == "self_debug_strategy"
                && x.strategy_signature == signature
        })
        .count()
}

fn collect_self_debug_runs(items: &[VmExecQueueItem]) -> Vec<VmSelfDebugRunSummary> {
    let mut map: BTreeMap<String, VmSelfDebugRunSummary> = BTreeMap::new();
    for item in items {
        let run_id = item.run_id.trim();
        if run_id.is_empty()
            || (item.run_kind != "self_debug"
                && item.run_kind != "self_debug_strategy"
                && item.run_kind != "self_debug_verify_after_strategy")
        {
            continue;
        }
        let entry = map.entry(run_id.to_string()).or_insert(VmSelfDebugRunSummary {
            run_id: run_id.to_string(),
            total: 0,
            pending: 0,
            paused: 0,
            running: 0,
            done: 0,
            failed: 0,
            canceled: 0,
            updated_at_unix: 0,
        });
        entry.total += 1;
        match item.status.as_str() {
            "pending" => entry.pending += 1,
            "paused" => entry.paused += 1,
            "running" => entry.running += 1,
            "done" => entry.done += 1,
            "failed" => entry.failed += 1,
            "canceled" => entry.canceled += 1,
            _ => {}
        }
        let t = item
            .finished_at_unix
            .max(item.started_at_unix)
            .max(item.created_at_unix);
        if t > entry.updated_at_unix {
            entry.updated_at_unix = t;
        }
    }
    let mut out: Vec<VmSelfDebugRunSummary> = map.into_values().collect();
    out.sort_by(|a, b| b.updated_at_unix.cmp(&a.updated_at_unix).then_with(|| b.run_id.cmp(&a.run_id)));
    out
}

fn load_self_debug_history(managed_root_dir: &str, vm_name: &str) -> Vec<VmSelfDebugHistoryEntry> {
    let path = vm_self_debug_history_path(managed_root_dir, vm_name);
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<VmSelfDebugHistoryEntry>>(&text).unwrap_or_default()
}

fn save_self_debug_history(
    managed_root_dir: &str,
    vm_name: &str,
    entries: &[VmSelfDebugHistoryEntry],
) -> std::io::Result<()> {
    let path = vm_self_debug_history_path(managed_root_dir, vm_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(entries)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn sort_history_entries(entries: &mut [VmSelfDebugHistoryEntry]) {
    entries.sort_by(|a, b| {
        b.archived_at_unix
            .cmp(&a.archived_at_unix)
            .then_with(|| b.summary.updated_at_unix.cmp(&a.summary.updated_at_unix))
            .then_with(|| b.summary.run_id.cmp(&a.summary.run_id))
    });
}

fn trim_history_entries(entries: &mut Vec<VmSelfDebugHistoryEntry>, max_entries: usize) {
    if max_entries == 0 || entries.len() <= max_entries {
        return;
    }
    entries.truncate(max_entries);
}

#[derive(Debug, Clone, Default)]
struct SelfDebugContextData {
    failed_steps: usize,
    categories: Vec<String>,
    key_lines: Vec<String>,
    context_text: String,
}

fn build_self_debug_context_data(items: &[VmExecQueueItem], run_id: &str) -> Option<SelfDebugContextData> {
    if run_id.trim().is_empty() {
        return None;
    }
    let mut failed_steps: Vec<VmExecQueueItem> = items
        .iter()
        .filter(|x| {
            is_self_debug_kind(&x.run_kind)
                && x.run_id == run_id
                && (x.status == "failed" || x.status == "canceled")
        })
        .cloned()
        .collect();
    if failed_steps.is_empty() {
        return None;
    }
    failed_steps.sort_by(|a, b| {
        a.created_at_unix
            .cmp(&b.created_at_unix)
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut category_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut key_lines: Vec<String> = Vec::new();
    for step in &failed_steps {
        let category = if step.failure_category.trim().is_empty() {
            "unknown_failure".to_string()
        } else {
            step.failure_category.trim().to_string()
        };
        *category_count.entry(category).or_insert(0) += 1;
        for line in &step.failure_key_lines {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if key_lines.iter().any(|x| x == t) {
                continue;
            }
            key_lines.push(t.to_string());
            if key_lines.len() >= 20 {
                break;
            }
        }
        if key_lines.len() >= 20 {
            break;
        }
    }

    let mut categories: Vec<String> = category_count
        .into_iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect();
    categories.sort();

    let mut lines = Vec::new();
    lines.push(format!("run_id={}", run_id));
    lines.push(format!("failed_steps={}", failed_steps.len()));
    lines.push(format!("categories={}", categories.join(", ")));
    lines.push(String::new());
    lines.push("recent_failed_steps:".to_string());
    for (idx, step) in failed_steps.iter().rev().take(8).enumerate() {
        lines.push(format!(
            "#{} kind={} status={} exit={} category={}",
            idx + 1,
            step.run_kind,
            step.status,
            step.exit_code,
            if step.failure_category.trim().is_empty() {
                "unknown_failure"
            } else {
                step.failure_category.as_str()
            }
        ));
        lines.push(format!("cmd={}", step.command));
        if !step.message.trim().is_empty() {
            lines.push(format!("msg={}", step.message.trim()));
        }
        if !step.output_preview.trim().is_empty() {
            lines.push(format!("out={}", tail_text(&step.output_preview, 500)));
        }
    }
    if !key_lines.is_empty() {
        lines.push(String::new());
        lines.push("key_failure_lines:".to_string());
        for line in &key_lines {
            lines.push(format!("- {}", line));
        }
    }
    Some(SelfDebugContextData {
        failed_steps: failed_steps.len(),
        categories,
        key_lines,
        context_text: lines.join("\n"),
    })
}

fn upsert_history_entry(
    history: &mut Vec<VmSelfDebugHistoryEntry>,
    summary: VmSelfDebugRunSummary,
    archived_at_unix: u64,
    ctx: SelfDebugContextData,
) {
    if let Some(existing) = history
        .iter_mut()
        .find(|x| x.summary.run_id == summary.run_id)
    {
        existing.summary = summary;
        existing.archived_at_unix = archived_at_unix;
        existing.failed_steps = ctx.failed_steps;
        existing.categories = ctx.categories;
        existing.key_lines = ctx.key_lines;
        existing.context_text = ctx.context_text;
        return;
    }
    history.push(VmSelfDebugHistoryEntry {
        summary,
        archived_at_unix,
        failed_steps: ctx.failed_steps,
        categories: ctx.categories,
        key_lines: ctx.key_lines,
        context_text: ctx.context_text,
    });
}

fn archive_completed_self_debug_runs(
    items: &mut Vec<VmExecQueueItem>,
    history: &mut Vec<VmSelfDebugHistoryEntry>,
    now: u64,
) -> (usize, usize) {
    let runs = collect_self_debug_runs(items);
    let completed_run_ids: BTreeSet<String> = runs
        .iter()
        .filter(|r| r.pending == 0 && r.paused == 0 && r.running == 0 && !r.run_id.trim().is_empty())
        .map(|r| r.run_id.clone())
        .collect();
    if completed_run_ids.is_empty() {
        return (0, 0);
    }
    let archived_runs: Vec<VmSelfDebugRunSummary> = runs
        .into_iter()
        .filter(|r| completed_run_ids.contains(&r.run_id))
        .collect();
    for summary in archived_runs {
        let ctx = build_self_debug_context_data(items, &summary.run_id).unwrap_or_default();
        upsert_history_entry(history, summary, now, ctx);
    }
    let before = items.len();
    items.retain(|x| !(is_self_debug_kind(&x.run_kind) && completed_run_ids.contains(&x.run_id)));
    let removed_tasks = before.saturating_sub(items.len());
    (completed_run_ids.len(), removed_tasks)
}

fn collect_self_debug_strategy_stats(items: &[VmExecQueueItem]) -> Vec<VmSelfDebugStrategyStat> {
    let mut trigger_category: BTreeMap<String, String> = BTreeMap::new();
    for item in items {
        if item.id.trim().is_empty() {
            continue;
        }
        if !item.failure_category.trim().is_empty() {
            trigger_category.insert(item.id.clone(), item.failure_category.clone());
        }
    }
    let mut agg: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();
    for item in items {
        if item.run_kind != "self_debug_verify_after_strategy" {
            continue;
        }
        let trigger = item.trigger_task_id.trim();
        let category = trigger_category
            .get(trigger)
            .cloned()
            .unwrap_or_else(|| "unknown_failure".to_string());
        let row = agg.entry(category).or_insert((0, 0, 0, 0));
        row.0 += 1; // attempts
        match item.status.as_str() {
            "done" => row.1 += 1,
            "failed" | "canceled" => row.2 += 1,
            "pending" | "running" => row.3 += 1,
            _ => {}
        }
    }
    let mut out: Vec<VmSelfDebugStrategyStat> = agg
        .into_iter()
        .map(|(category, (attempts, ok, fail, pending))| {
            let denom = ok + fail;
            let rate = if denom == 0 {
                0.0
            } else {
                (ok as f64) * 100.0 / (denom as f64)
            };
            VmSelfDebugStrategyStat {
                category,
                attempts,
                verified_success: ok,
                verified_fail: fail,
                pending,
                success_rate: rate,
            }
        })
        .collect();
    out.sort_by(|a, b| b.attempts.cmp(&a.attempts).then_with(|| a.category.cmp(&b.category)));
    out
}

fn strategy_priority_boost_by_stats(items: &[VmExecQueueItem], category: &str) -> i32 {
    let stats = collect_self_debug_strategy_stats(items);
    let Some(row) = stats.into_iter().find(|x| x.category == category) else {
        return 0;
    };
    if row.attempts < 3 {
        return 0;
    }
    let rate = row.success_rate;
    if rate >= 80.0 {
        3
    } else if rate >= 60.0 {
        2
    } else if rate >= 40.0 {
        1
    } else {
        0
    }
}

fn is_self_debug_kind(kind: &str) -> bool {
    kind == "self_debug" || kind == "self_debug_strategy" || kind == "self_debug_verify_after_strategy"
}

fn enforce_self_debug_run_timeout(
    items: &mut [VmExecQueueItem],
    run_id: &str,
    now: u64,
) -> Option<u64> {
    if run_id.trim().is_empty() {
        return None;
    }
    let mut min_created = u64::MAX;
    let mut max_runtime = 0u64;
    for item in items.iter() {
        if item.run_id != run_id || !is_self_debug_kind(&item.run_kind) {
            continue;
        }
        min_created = min_created.min(item.created_at_unix);
        max_runtime = max_runtime.max(item.run_max_runtime_sec);
    }
    if max_runtime == 0 || min_created == u64::MAX {
        return None;
    }
    if now.saturating_sub(min_created) < max_runtime {
        return None;
    }
    let mut changed = 0usize;
    for item in items.iter_mut() {
        if item.run_id != run_id || !is_self_debug_kind(&item.run_kind) {
            continue;
        }
        if item.status == "pending" || item.status == "paused" {
            item.status = "canceled".to_string();
            item.finished_at_unix = now;
            item.exit_code = -2;
            item.message = "canceled by run max_runtime timeout".to_string();
            changed += 1;
        }
    }
    if changed == 0 {
        return None;
    }
    Some(max_runtime)
}

fn normalize_batch_task(
    task: &VmExecBatchTaskPayload,
    default_timeout_sec: u64,
    default_wait_ready_sec: u64,
    default_priority: i32,
    default_retry_max: u32,
) -> Option<VmExecQueueItem> {
    let cmd = task.command.trim();
    if cmd.is_empty() {
        return None;
    }
    let timeout_sec = if task.timeout_sec == 0 {
        default_timeout_sec
    } else {
        task.timeout_sec.clamp(1, 3600)
    };
    let wait_ready_sec = if task.wait_ready_sec == 0 {
        default_wait_ready_sec
    } else {
        task.wait_ready_sec.clamp(0, 600)
    };
    let priority = if task.priority == 0 {
        default_priority
    } else {
        task.priority.clamp(-100, 100)
    };
    let retry_max = if task.retry_max == 0 {
        default_retry_max
    } else {
        task.retry_max.clamp(0, 10)
    };
    Some(queue_item_from_values(
        cmd,
        timeout_sec,
        wait_ready_sec,
        priority,
        retry_max,
    ))
}

fn vm_exec_profiles() -> &'static [&'static str] {
    &[
        "rust-self-debug-basic",
        "python-self-debug-basic",
        "node-self-debug-basic",
    ]
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct VmExecCustomProfilesStore {
    #[serde(default)]
    profiles: BTreeMap<String, Vec<String>>,
}

fn load_custom_profiles(managed_root_dir: &str) -> VmExecCustomProfilesStore {
    let path = vm_profiles_path(managed_root_dir);
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return VmExecCustomProfilesStore::default(),
    };
    serde_json::from_str::<VmExecCustomProfilesStore>(&text).unwrap_or_default()
}

fn save_custom_profiles(
    managed_root_dir: &str,
    store: &VmExecCustomProfilesStore,
) -> std::io::Result<()> {
    let path = vm_profiles_path(managed_root_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(store)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn is_queue_active_status(status: &str) -> bool {
    matches!(status, "pending" | "running" | "paused")
}

fn queue_has_active_duplicate(items: &[VmExecQueueItem], command: &str) -> bool {
    let target = command.trim();
    if target.is_empty() {
        return false;
    }
    items
        .iter()
        .any(|x| is_queue_active_status(&x.status) && x.command.trim() == target)
}

fn priority_aging_boost(created_at_unix: u64, now: u64, step_sec: u64, max_boost: i32) -> i32 {
    if created_at_unix == 0 || now <= created_at_unix {
        return 0;
    }
    let waited = now.saturating_sub(created_at_unix);
    let steps = (waited / step_sec.max(1)) as i32;
    steps.clamp(0, max_boost.max(0))
}

fn effective_priority_with_aging(
    item: &VmExecQueueItem,
    now: u64,
    step_sec: u64,
    max_boost: i32,
) -> i32 {
    item.priority
        .saturating_add(priority_aging_boost(item.created_at_unix, now, step_sec, max_boost))
        .clamp(-100, 100)
}

fn sanitize_custom_profile_name(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.len() < 8 || s.len() > 64 {
        return None;
    }
    if !s.starts_with("custom-") {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
    {
        return None;
    }
    Some(s.to_string())
}

fn normalize_custom_commands(raw: &[String]) -> Vec<String> {
    raw.iter()
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty() && x.len() <= 400)
        .take(40)
        .collect()
}

fn list_all_profiles(managed_root_dir: &str) -> Vec<String> {
    let mut all: Vec<String> = vm_exec_profiles().iter().map(|x| (*x).to_string()).collect();
    let store = load_custom_profiles(managed_root_dir);
    all.extend(store.profiles.keys().cloned());
    all.sort();
    all.dedup();
    all
}

#[derive(Clone, Debug, Default)]
struct ProfileRuntimeOptions {
    workdir: Option<String>,
    test_cmd: Option<String>,
}

fn normalize_profile_workdir(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.len() > 200 {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
    {
        return None;
    }
    if !(s.starts_with('/') || s.starts_with('.')) {
        return None;
    }
    Some(s.to_string())
}

fn normalize_profile_test_cmd(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.len() > 300 {
        return None;
    }
    Some(s.to_string())
}

fn parse_profile_options(workdir: &str, test_cmd: &str) -> Result<ProfileRuntimeOptions, String> {
    let wd_raw = workdir.trim();
    let wd = normalize_profile_workdir(wd_raw);
    if !wd_raw.is_empty() && wd.is_none() {
        return Err("invalid workdir: only [a-zA-Z0-9_./-], must start with / or .".to_string());
    }
    Ok(ProfileRuntimeOptions {
        workdir: wd,
        test_cmd: normalize_profile_test_cmd(test_cmd),
    })
}

fn prepend_workdir(cmd: &str, workdir: Option<&str>) -> String {
    match workdir {
        Some(w) if !w.is_empty() => format!("cd {} && {}", w, cmd),
        _ => cmd.to_string(),
    }
}

fn build_builtin_profile_batch_tasks(
    profile: &str,
    opts: &ProfileRuntimeOptions,
) -> Option<Vec<VmExecBatchTaskPayload>> {
    let test_cmd_rust = opts
        .test_cmd
        .clone()
        .unwrap_or_else(|| "bash scripts/run_tests.sh".to_string());
    let test_cmd_python = opts
        .test_cmd
        .clone()
        .unwrap_or_else(|| "python3 -m pytest -q".to_string());
    let test_cmd_node = opts
        .test_cmd
        .clone()
        .unwrap_or_else(|| "npm test -- --watch=false".to_string());
    let wd = opts.workdir.as_deref();
    match profile.trim() {
        "rust-self-debug-basic" => Some(vec![
            VmExecBatchTaskPayload {
                command: prepend_workdir("pwd", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir("git status --short", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir(&test_cmd_rust, wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir("cargo check", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir("cargo test -q", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
        ]),
        "python-self-debug-basic" => Some(vec![
            VmExecBatchTaskPayload {
                command: prepend_workdir("pwd", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir("git status --short", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir("python3 -m pip --version", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir(&test_cmd_python, wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
        ]),
        "node-self-debug-basic" => Some(vec![
            VmExecBatchTaskPayload {
                command: prepend_workdir("pwd", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir("git status --short", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir("npm --version", wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
            VmExecBatchTaskPayload {
                command: prepend_workdir(&test_cmd_node, wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            },
        ]),
        _ => None,
    }
}

fn build_profile_batch_tasks(
    managed_root_dir: &str,
    profile: &str,
    opts: &ProfileRuntimeOptions,
) -> Option<Vec<VmExecBatchTaskPayload>> {
    if let Some(v) = build_builtin_profile_batch_tasks(profile, opts) {
        return Some(v);
    }
    let store = load_custom_profiles(managed_root_dir);
    let raw = store.profiles.get(profile)?;
    let wd = opts.workdir.as_deref();
    let out: Vec<VmExecBatchTaskPayload> = raw
        .iter()
        .map(|cmd| VmExecBatchTaskPayload {
            command: prepend_workdir(cmd, wd),
            timeout_sec: 0,
            wait_ready_sec: 0,
            priority: 0,
            retry_max: 0,
        })
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn default_test_cmd_for_profile(profile: &str) -> &'static str {
    match profile.trim() {
        "python-self-debug-basic" => "python3 -m pytest -q",
        "node-self-debug-basic" => "npm test -- --watch=false",
        _ => "bash scripts/run_tests.sh",
    }
}

fn build_self_debug_tasks(
    body: &VmSelfDebugPlanPayload,
) -> Result<Vec<VmExecBatchTaskPayload>, String> {
    let profile = if body.profile.trim().is_empty() {
        "rust-self-debug-basic".to_string()
    } else {
        body.profile.trim().to_string()
    };
    let cycles = if body.cycles == 0 {
        3
    } else {
        body.cycles.clamp(1, 30)
    };
    let workdir = normalize_profile_workdir(&body.workdir);
    if !body.workdir.trim().is_empty() && workdir.is_none() {
        return Err("invalid workdir: only [a-zA-Z0-9_./-], must start with / or .".to_string());
    }
    let test_cmd = if body.test_cmd.trim().is_empty() {
        default_test_cmd_for_profile(&profile).to_string()
    } else {
        body.test_cmd.trim().to_string()
    };
    let fix_cmd = body.fix_cmd.trim();
    if fix_cmd.is_empty() {
        return Err("fix_cmd is required for self-debug plan".to_string());
    }
    let verify_cmd = if body.verify_cmd.trim().is_empty() {
        test_cmd.clone()
    } else {
        body.verify_cmd.trim().to_string()
    };
    let success_target = if body.success_streak_target == 0 {
        2
    } else {
        body.success_streak_target.clamp(1, 10)
    };
    let fail_target = if body.fail_streak_target == 0 {
        3
    } else {
        body.fail_streak_target.clamp(1, 10)
    };
    let wd = workdir.as_deref();
    let mut out = Vec::new();
    out.push(VmExecBatchTaskPayload {
        command: prepend_workdir(
            "sh -lc \"mkdir -p .kacf && echo 0 > .kacf/self_debug_success_streak && echo 0 > .kacf/self_debug_fail_streak\"",
            wd,
        ),
        timeout_sec: 0,
        wait_ready_sec: 0,
        priority: 0,
        retry_max: 0,
    });
    for i in 1..=cycles {
        let start_line = format!("echo '[self-debug] cycle {i}/{cycles} start'");
        let end_line = format!("echo '[self-debug] cycle {i}/{cycles} end'");
        let verify_gate = format!(
            "sh -lc '{} && s=$(cat .kacf/self_debug_success_streak 2>/dev/null || echo 0); f=$(cat .kacf/self_debug_fail_streak 2>/dev/null || echo 0); s=$((s+1)); f=0; echo $s > .kacf/self_debug_success_streak; echo $f > .kacf/self_debug_fail_streak; echo \"[self-debug] verify success streak=$s/{}\"; if [ \"$s\" -ge \"{}\" ]; then echo \"[self-debug] success streak reached, request early stop\"; touch .kacf/self_debug_stop_request; fi' || sh -lc 's=$(cat .kacf/self_debug_success_streak 2>/dev/null || echo 0); f=$(cat .kacf/self_debug_fail_streak 2>/dev/null || echo 0); s=0; f=$((f+1)); echo $s > .kacf/self_debug_success_streak; echo $f > .kacf/self_debug_fail_streak; echo \"[self-debug] verify fail streak=$f/{}\"; if [ \"$f\" -ge \"{}\" ]; then echo \"[self-debug] fail streak reached, request early stop\"; touch .kacf/self_debug_stop_request; fi; exit 0'",
            verify_cmd.replace('\'', "'\"'\"'"),
            success_target,
            success_target,
            fail_target,
            fail_target
        );
        let check_stop = "sh -lc \"if [ -f .kacf/self_debug_stop_request ]; then echo '[self-debug] early-stop flag detected'; exit 10; fi\"".to_string();
        let cmds = [
            start_line.as_str(),
            test_cmd.as_str(),
            fix_cmd,
            verify_gate.as_str(),
            check_stop.as_str(),
            end_line.as_str(),
        ];
        for cmd in cmds {
            out.push(VmExecBatchTaskPayload {
                command: prepend_workdir(cmd, wd),
                timeout_sec: 0,
                wait_ready_sec: 0,
                priority: 0,
                retry_max: 0,
            });
        }
    }
    Ok(out)
}

fn try_acquire_worker_lock(managed_root_dir: &str, vm_name: &str) -> bool {
    let lock_path = vm_exec_worker_lock_path(managed_root_dir, vm_name);
    if let Some(parent) = lock_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&lock_path)
    {
        Ok(mut f) => {
            let _ = std::io::Write::write_all(&mut f, format!("pid={}\n", std::process::id()).as_bytes());
            true
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => false,
        Err(_) => false,
    }
}

fn release_worker_lock(managed_root_dir: &str, vm_name: &str) {
    let _ = fs::remove_file(vm_exec_worker_lock_path(managed_root_dir, vm_name));
}

fn run_next_vm_exec_core(
    managed_root_dir: &str,
    name: &str,
    projects_lock: &Arc<Mutex<()>>,
    running_limit: usize,
) -> Result<Option<VmExecResponse>, String> {
    let (task_id, cmd, cmd_risk_level, cmd_risk_tags, timeout_sec, wait_ready_sec, ssh_user, ssh_port) = {
        let _guard = lock_recover(projects_lock, "projects_lock");
        let mut state = load_vm_state(managed_root_dir);
        if !state.vms.contains_key(name) {
            return Err("vm not found".to_string());
        }
        let recovered = recover_stale_running_tasks_all_vms(managed_root_dir, &state, now_unix());
        if recovered > 0 {
            append_vm_log(
                managed_root_dir,
                name,
                &format!("exec watchdog recovered total_stale_tasks={recovered}"),
            );
        }
        let (ssh_user, ssh_port) = {
            let Some(vm) = state.vms.get_mut(name) else {
                return Err("vm not found".to_string());
            };
            reconcile_vm_power_state(managed_root_dir, vm);
            if vm.backend != "qemu" {
                return Err("vm exec queue only supports qemu backend".to_string());
            }
            if vm.power_state != "running" {
                return Err("vm is not running".to_string());
            }
            let ssh_port = vm.ssh_port.unwrap_or(0);
            if ssh_port == 0 {
                return Err("vm ssh port is not set".to_string());
            }
            (normalize_ssh_user(&vm.ssh_user), ssh_port)
        };
        let mut items = load_exec_queue(managed_root_dir, name);
        if items.iter().any(|x| x.status == "running") {
            return Ok(None);
        }
        let now = now_unix();
        let scheduler = load_vm_policy_config(managed_root_dir).scheduler;
        let pending_idx = items
            .iter()
            .enumerate()
            .filter(|(_, x)| x.status == "pending" && now >= x.next_run_after_unix)
            .max_by(|(ia, a), (ib, b)| {
                effective_priority_with_aging(
                    a,
                    now,
                    scheduler.priority_aging_step_sec,
                    scheduler.priority_aging_max_boost,
                )
                .cmp(&effective_priority_with_aging(
                    b,
                    now,
                    scheduler.priority_aging_step_sec,
                    scheduler.priority_aging_max_boost,
                ))
                    .then_with(|| b.created_at_unix.cmp(&a.created_at_unix))
                    .then_with(|| ib.cmp(ia))
            })
            .map(|(idx, _)| idx);
        let Some(idx) = pending_idx else {
            return Ok(None);
        };
        let running_total = count_running_exec_tasks_all_vms(managed_root_dir, &state);
        if running_total >= running_limit {
            append_vm_log(
                managed_root_dir,
                name,
                &format!(
                    "exec concurrency limited: running_total={} limit={}",
                    running_total, running_limit
                ),
            );
            return Ok(None);
        }
        items[idx].status = "running".to_string();
        items[idx].started_at_unix = now_unix();
        let task_id = items[idx].id.clone();
        let cmd = items[idx].command.clone();
        let cmd_risk_level = items[idx].command_risk_level.clone();
        let cmd_risk_tags = items[idx].command_risk_tags.clone();
        let timeout_sec = if items[idx].timeout_sec == 0 {
            30
        } else {
            items[idx].timeout_sec.clamp(1, 3600)
        };
        let wait_ready_sec = items[idx].wait_ready_sec.clamp(0, 600);
        save_exec_queue(managed_root_dir, name, &items)
            .map_err(|e| format!("save queue failed: {e}"))?;
        (
            task_id,
            cmd,
            cmd_risk_level,
            cmd_risk_tags,
            timeout_sec,
            wait_ready_sec,
            ssh_user,
            ssh_port,
        )
    };
    append_vm_log(
        managed_root_dir,
        name,
        &format!(
            "queue task running task_id={} risk_level={} risk_tags={}",
            task_id,
            cmd_risk_level,
            if cmd_risk_tags.is_empty() {
                "-".to_string()
            } else {
                cmd_risk_tags.join(",")
            }
        ),
    );

    if let Err(e) = wait_until_ssh_ready(&ssh_user, ssh_port, wait_ready_sec) {
        let _guard = lock_recover(projects_lock, "projects_lock");
        let mut items = load_exec_queue(managed_root_dir, name);
        if let Some(item) = items.iter_mut().find(|x| x.id == task_id) {
            if item.retry_count < item.retry_max {
                item.retry_count += 1;
                item.status = "pending".to_string();
                item.started_at_unix = 0;
                item.finished_at_unix = 0;
                item.exit_code = -1;
                item.next_run_after_unix = now_unix() + retry_backoff_secs(item.retry_count);
                item.message = format!(
                    "ssh not ready, auto-retry {}/{} after {}s",
                    item.retry_count,
                    item.retry_max,
                    retry_backoff_secs(item.retry_count)
                );
            } else {
                item.status = "failed".to_string();
                item.finished_at_unix = now_unix();
                item.exit_code = -1;
                item.message = e.clone();
            }
        }
        let _ = save_exec_queue(managed_root_dir, name, &items);
        return Ok(Some(VmExecResponse {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            message: e,
        }));
    }

    let pid_path = vm_exec_pid_path(managed_root_dir, name);
    let cancel_path = vm_exec_cancel_path(managed_root_dir, name);
    let exec_resp = match run_ssh_command_controlled(
        &ssh_user,
        ssh_port,
        &cmd,
        timeout_sec,
        Some(&pid_path),
        Some(&cancel_path),
    ) {
        Ok(v) => v,
        Err(e) => VmExecResponse {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            message: e,
        },
    };

    let _guard = lock_recover(projects_lock, "projects_lock");
    let mut items = load_exec_queue(managed_root_dir, name);
    let mut finished_run_id = String::new();
    let mut finished_run_kind = String::new();
    let mut strategy_candidate: Option<(String, String, i32, String, String, String)> = None;
    if let Some(item) = items.iter_mut().find(|x| x.id == task_id) {
        item.finished_at_unix = now_unix();
        item.exit_code = exec_resp.exit_code;
        item.message = exec_resp.message.clone();
        item.output_preview = build_exec_output_preview(&exec_resp.stdout, &exec_resp.stderr, 1200);
        let (cat, sig, lines) = classify_failure(
            exec_resp.exit_code,
            &exec_resp.stderr,
            &exec_resp.stdout,
            &exec_resp.message,
        );
        item.failure_category = cat;
        item.failure_signature = sig;
        item.failure_key_lines = lines;
        item.next_run_after_unix = 0;
        finished_run_id = item.run_id.clone();
        finished_run_kind = item.run_kind.clone();
        item.status = if exec_resp.exit_code == -2 {
            "canceled".to_string()
        } else if exec_resp.exit_code == 10 && item.run_kind == "self_debug" {
            "done".to_string()
        } else if exec_resp.ok {
            "done".to_string()
        } else if item.retry_count < item.retry_max {
            item.retry_count += 1;
            item.started_at_unix = 0;
            item.finished_at_unix = 0;
            item.next_run_after_unix = now_unix() + retry_backoff_secs(item.retry_count);
            item.message = format!(
                "attempt failed (exit={}); auto-retry {}/{} after {}s",
                exec_resp.exit_code,
                item.retry_count,
                item.retry_max,
                retry_backoff_secs(item.retry_count)
            );
            "pending".to_string()
        } else {
            "failed".to_string()
        };
        if item.status == "failed"
            && item.run_kind == "self_debug"
            && !item.run_id.trim().is_empty()
        {
            strategy_candidate = Some((
                item.run_id.clone(),
                item.failure_category.clone(),
                item.priority,
                item.failure_signature.clone(),
                item.id.clone(),
                item.command.clone(),
            ));
        }
    }
    if exec_resp.exit_code == 10
        && finished_run_kind == "self_debug"
        && !finished_run_id.trim().is_empty()
    {
        for item in items.iter_mut() {
            if item.id == task_id {
                continue;
            }
            if item.run_kind != "self_debug" || item.run_id != finished_run_id {
                continue;
            }
            if item.status == "pending" {
                item.status = "canceled".to_string();
                item.finished_at_unix = now_unix();
                item.exit_code = -2;
                item.message = "canceled by early-stop gate".to_string();
            }
        }
        append_vm_log(
            managed_root_dir,
            name,
            &format!("self-debug early-stop gate triggered run_id={}", finished_run_id),
        );
    }
    let run_timed_out = if !finished_run_id.trim().is_empty() && is_self_debug_kind(&finished_run_kind) {
        let now = now_unix();
        let hit = enforce_self_debug_run_timeout(&mut items, &finished_run_id, now);
        if let Some(max_runtime) = hit {
            append_vm_log(
                managed_root_dir,
                name,
                &format!(
                    "self-debug run timeout reached run_id={} max_runtime_sec={}",
                    finished_run_id, max_runtime
                ),
            );
            true
        } else {
            false
        }
    } else {
        false
    };
    let inject_strategy = strategy_candidate.as_ref().and_then(|c| {
        if run_timed_out {
            return None;
        }
        let current = strategy_injected_count(&items, &c.0, &c.3);
        let max_inject = 2usize;
        if current < max_inject {
            Some(c.clone())
        } else {
            None
        }
    });
    if let Some((run_id, category, priority, signature, trigger_task_id, failed_command)) = inject_strategy {
        let inherited_run_max_runtime = items
            .iter()
            .find(|x| x.id == trigger_task_id)
            .map(|x| x.run_max_runtime_sec)
            .unwrap_or(0);
        let dynamic_boost = strategy_priority_boost_by_stats(&items, &category);
        let strategy_priority = priority
            .saturating_add(1)
            .saturating_add(dynamic_boost)
            .clamp(-100, 100);
        if let Some(item) = items.iter_mut().find(|x| x.id == trigger_task_id) {
            item.message = format!("{} [strategy-injected]", item.message);
        }
        let mut strategy_item = queue_item_from_values(
            &strategy_command_for_failure_category(managed_root_dir, &category),
            180,
            0,
            strategy_priority,
            0,
        );
        strategy_item.run_id = run_id.clone();
        strategy_item.run_kind = "self_debug_strategy".to_string();
        strategy_item.run_max_runtime_sec = inherited_run_max_runtime;
        strategy_item.strategy_signature = signature.clone();
        strategy_item.trigger_task_id = trigger_task_id.clone();
        strategy_item.message = format!(
            "auto strategy task injected for category={} priority_boost={}",
            category, dynamic_boost
        );
        items.push(strategy_item);

        let verify_after_cmd = if failed_command.trim().is_empty() {
            "bash scripts/run_tests.sh".to_string()
        } else {
            failed_command
        };
        let mut verify_after_item = queue_item_from_values(
            &verify_after_cmd,
            180,
            0,
            strategy_priority,
            0,
        );
        verify_after_item.run_id = run_id.clone();
        verify_after_item.run_kind = "self_debug_verify_after_strategy".to_string();
        verify_after_item.run_max_runtime_sec = inherited_run_max_runtime;
        verify_after_item.strategy_signature = signature.clone();
        verify_after_item.trigger_task_id = trigger_task_id.clone();
        verify_after_item.message = "auto verify-after-strategy task injected".to_string();
        items.push(verify_after_item);
        append_vm_log(
            managed_root_dir,
            name,
            &format!(
                "self-debug strategy injected run_id={} category={} signature={}",
                run_id, category, signature
            ),
        );
    } else if let Some((_, _, _, _, trigger_task_id, _)) = strategy_candidate {
        if let Some(item) = items.iter_mut().find(|x| x.id == trigger_task_id) {
            item.message = format!("{} [strategy-skip-limit]", item.message);
        }
    }
    let now = now_unix();
    let mut history = load_self_debug_history(managed_root_dir, name);
    let (archived_runs, removed_tasks) =
        archive_completed_self_debug_runs(&mut items, &mut history, now);
    sort_history_entries(&mut history);
    trim_history_entries(&mut history, VM_HISTORY_MAX_ENTRIES);
    let _ = save_exec_queue(managed_root_dir, name, &items);
    let _ = save_self_debug_history(managed_root_dir, name, &history);
    if archived_runs > 0 {
        append_vm_log(
            managed_root_dir,
            name,
            &format!(
                "self-debug auto-archive archived_runs={} removed_tasks={} history_total={}",
                archived_runs,
                removed_tasks,
                history.len()
            ),
        );
    }
    append_vm_log(
        managed_root_dir,
        name,
        &format!("queue run finished task_id={} exit={}", task_id, exec_resp.exit_code),
    );
    Ok(Some(exec_resp))
}

fn spawn_vm_exec_worker(
    managed_root_dir: String,
    name: String,
    projects_lock: Arc<Mutex<()>>,
    running_limit: usize,
) {
    if !try_acquire_worker_lock(&managed_root_dir, &name) {
        return;
    }
    thread::spawn(move || {
        let mut ran_one = false;
        match run_next_vm_exec_core(&managed_root_dir, &name, &projects_lock, running_limit) {
            Ok(Some(_)) => ran_one = true,
            Ok(None) => {}
            Err(e) => {
                append_vm_log(&managed_root_dir, &name, &format!("queue worker stopped: {e}"));
            }
        }
        release_worker_lock(&managed_root_dir, &name);
        if ran_one {
            let _ = dispatch_vm_exec_workers(&managed_root_dir, projects_lock.clone(), running_limit);
        }
    });
}

fn dispatch_vm_exec_workers(
    managed_root_dir: &str,
    projects_lock: Arc<Mutex<()>>,
    running_limit: usize,
) -> usize {
    let (vm_names, trace_entry): (Vec<String>, VmExecDispatchTraceEntry) = {
        let _guard = lock_recover(&projects_lock, "projects_lock");
        let state = load_vm_state(managed_root_dir);
        let running_total = count_running_exec_tasks_all_vms(managed_root_dir, &state);
        if running_total >= running_limit {
            (
                Vec::new(),
                VmExecDispatchTraceEntry {
                    created_at_unix: now_unix(),
                    running_total_before: running_total,
                    running_limit,
                    available_slots: 0,
                    selected_vms: Vec::new(),
                    candidates: Vec::new(),
                },
            )
        } else {
            let slots = running_limit.saturating_sub(running_total);
            let now = now_unix();
            let scores = select_dispatch_vm_candidates_with_score(managed_root_dir, &state, now);
            let selected_vms: Vec<String> = scores
                .iter()
                .take(slots)
                .map(|x| x.vm_name.clone())
                .collect();
            let candidates = scores
                .iter()
                .map(|x| VmExecDispatchTraceCandidate {
                    vm_name: x.vm_name.clone(),
                    base_priority: x.base_priority,
                    effective_priority: x.effective_priority,
                    oldest_pending_age_sec: x.oldest_pending_age_sec,
                    selected: selected_vms.iter().any(|v| v == &x.vm_name),
                })
                .collect();
            (
                selected_vms.clone(),
                VmExecDispatchTraceEntry {
                    created_at_unix: now,
                    running_total_before: running_total,
                    running_limit,
                    available_slots: slots,
                    selected_vms,
                    candidates,
                },
            )
        }
    };
    append_dispatch_trace_entry(managed_root_dir, trace_entry);
    let mut started = 0usize;
    for vm_name in vm_names {
        spawn_vm_exec_worker(
            managed_root_dir.to_string(),
            vm_name,
            projects_lock.clone(),
            running_limit,
        );
        started += 1;
    }
    started
}

#[derive(Debug, Clone)]
struct DispatchCandidateScore {
    vm_name: String,
    base_priority: i32,
    effective_priority: i32,
    oldest_pending_age_sec: u64,
    oldest_created: u64,
}

fn select_dispatch_vm_candidates_with_score(
    managed_root_dir: &str,
    state: &VmStateStore,
    now: u64,
) -> Vec<DispatchCandidateScore> {
    let scheduler = load_vm_policy_config(managed_root_dir).scheduler;
    let mut candidates: Vec<DispatchCandidateScore> = Vec::new();
    for vm_name in state.vms.keys() {
        let items = load_exec_queue(managed_root_dir, vm_name);
        if items.iter().any(|x| x.status == "running") {
            continue;
        }
        let mut best_base_priority = -101;
        let mut best_effective_priority = -101;
        let mut oldest_created = u64::MAX;
        for item in &items {
            if item.status != "pending" || now < item.next_run_after_unix {
                continue;
            }
            best_base_priority = best_base_priority.max(item.priority);
            best_effective_priority = best_effective_priority.max(effective_priority_with_aging(
                item,
                now,
                scheduler.priority_aging_step_sec,
                scheduler.priority_aging_max_boost,
            ));
            oldest_created = oldest_created.min(item.created_at_unix);
        }
        if best_effective_priority >= -100 {
            let age = if oldest_created == u64::MAX {
                0
            } else {
                now.saturating_sub(oldest_created)
            };
            candidates.push(DispatchCandidateScore {
                vm_name: vm_name.clone(),
                base_priority: best_base_priority,
                effective_priority: best_effective_priority,
                oldest_pending_age_sec: age,
                oldest_created,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.effective_priority
            .cmp(&a.effective_priority)
            .then_with(|| a.oldest_created.cmp(&b.oldest_created))
            .then_with(|| a.vm_name.cmp(&b.vm_name))
    });
    candidates
}

fn scan_vm_health_locked(
    managed_root_dir: &str,
    state: &mut VmStateStore,
    self_heal: bool,
) -> (Vec<VmHealthIssue>, Vec<VmHealthAction>) {
    let mut issues: Vec<VmHealthIssue> = Vec::new();
    let mut actions: Vec<VmHealthAction> = Vec::new();
    let now = now_unix();
    let grace_sec = load_vm_policy_config(managed_root_dir)
        .scheduler
        .exec_hard_timeout_grace_sec;
    for vm in state.vms.values_mut() {
        let vm_name = vm.name.clone();
        let before_power = vm.power_state.clone();
        let _ = reconcile_vm_power_state(managed_root_dir, vm);
        if before_power != vm.power_state {
            issues.push(VmHealthIssue {
                vm_name: vm_name.clone(),
                severity: "warn".to_string(),
                message: format!(
                    "power state reconciled: {} -> {}",
                    before_power, vm.power_state
                ),
            });
        }
        if vm.backend == "qemu" && vm.power_state == "running" && vm.ssh_port.unwrap_or(0) == 0 {
            issues.push(VmHealthIssue {
                vm_name: vm_name.clone(),
                severity: "error".to_string(),
                message: "running qemu vm has empty ssh_port".to_string(),
            });
        }
        let mut items = load_exec_queue(managed_root_dir, &vm_name);
        let stale_count = items
            .iter()
            .filter(|x| is_running_task_hard_timed_out(x, now, grace_sec))
            .count();
        if stale_count > 0 {
            issues.push(VmHealthIssue {
                vm_name: vm_name.clone(),
                severity: "error".to_string(),
                message: format!("stale running exec tasks detected: {stale_count}"),
            });
            if self_heal {
                let recovered = recover_stale_running_tasks_in_queue(&mut items, now, grace_sec);
                if recovered > 0 {
                    force_stop_vm_exec_process(managed_root_dir, &vm_name);
                    let _ = save_exec_queue(managed_root_dir, &vm_name, &items);
                    append_vm_log(
                        managed_root_dir,
                        &vm_name,
                        &format!("health-scan self-heal recovered stale tasks={recovered}"),
                    );
                    actions.push(VmHealthAction {
                        vm_name: vm_name.clone(),
                        action: "recover_stale_exec_tasks".to_string(),
                        detail: format!("recovered={recovered}"),
                    });
                }
            }
        }
    }
    (issues, actions)
}

fn load_vm_state(managed_root_dir: &str) -> VmStateStore {
    let path = vm_state_path(managed_root_dir);
    let content = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return VmStateStore::default(),
    };
    serde_json::from_str::<VmStateStore>(&content).unwrap_or_default()
}

fn save_vm_state(managed_root_dir: &str, payload: &VmStateStore) -> std::io::Result<()> {
    let path = vm_state_path(managed_root_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(payload)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn command_available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

fn probe_capabilities() -> Vec<VmCapability> {
    let qemu_sys = command_available("qemu-system-x86_64");
    let qemu_img = command_available("qemu-img");
    let virsh = command_available("virsh");
    vec![
        VmCapability {
            backend: "qemu".to_string(),
            available: qemu_sys && qemu_img,
            detail: format!("qemu-system-x86_64={} qemu-img={}", qemu_sys, qemu_img),
        },
        VmCapability {
            backend: "libvirt".to_string(),
            available: virsh,
            detail: format!("virsh={}", virsh),
        },
        VmCapability {
            backend: "metadata-only".to_string(),
            available: true,
            detail: "always available; lifecycle only updates stored VM state".to_string(),
        },
    ]
}

fn sanitize_vm_name(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    Some(s.to_string())
}

fn sanitize_snapshot_name(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    Some(s.to_string())
}

fn normalize_ssh_user(raw: &str) -> String {
    let v = raw.trim();
    if v.is_empty() {
        return "root".to_string();
    }
    if v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return v.to_string();
    }
    "root".to_string()
}

fn allocate_local_port() -> Option<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    if port == 0 { None } else { Some(port) }
}

fn normalize_backend(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "qemu" => "qemu",
        "libvirt" => "libvirt",
        _ => "metadata-only",
    }
}

fn default_cpu(v: u32) -> u32 {
    if v == 0 {
        2
    } else {
        v.clamp(1, 32)
    }
}

fn default_memory_mb(v: u32) -> u32 {
    if v == 0 {
        4096
    } else {
        v.clamp(512, 262_144)
    }
}

fn default_disk_gb(v: u32) -> u32 {
    if v == 0 {
        40
    } else {
        v.clamp(10, 2048)
    }
}

fn create_disk_image(path: &Path, disk_gb: u32) -> std::io::Result<String> {
    if command_available("qemu-img") {
        let size = format!("{disk_gb}G");
        let out = Command::new("qemu-img")
            .args(["create", "-f", "qcow2"])
            .arg(path)
            .arg(size)
            .output();
        match out {
            Ok(o) if o.status.success() => {
                return Ok("disk image created by qemu-img (qcow2)".to_string());
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
                return Err(std::io::Error::other(format!("qemu-img create failed: {stderr}")));
            }
            Err(e) => {
                return Err(std::io::Error::other(format!("qemu-img spawn failed: {e}")));
            }
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(path)?;
    file.set_len(disk_gb as u64 * 1024 * 1024 * 1024)?;
    Ok("qemu-img unavailable, created sparse raw disk file".to_string())
}

fn pid_alive(pid: u32) -> bool {
    match Command::new("kill").args(["-0", &pid.to_string()]).status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

fn stop_pid(pid: u32) -> bool {
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    for _ in 0..20 {
        if !pid_alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
    !pid_alive(pid)
}

fn read_pid_file(path: &Path) -> Option<u32> {
    let txt = fs::read_to_string(path).ok()?;
    txt.trim().parse::<u32>().ok()
}

fn abs_path_from_rel_or_abs(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        p
    } else {
        workspace_root().join(p)
    }
}

fn resolve_iso_path(raw: &str) -> Result<Option<PathBuf>, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let p = abs_path_from_rel_or_abs(text);
    if !p.exists() {
        return Err(format!("os image not found: {}", p.display()));
    }
    if !p.is_file() {
        return Err(format!("os image is not a file: {}", p.display()));
    }
    Ok(Some(p))
}

fn start_qemu_process(
    managed_root_dir: &str,
    vm: &VmInstance,
    ssh_port: u16,
) -> Result<(u32, String), String> {
    if !command_available("qemu-system-x86_64") {
        return Err("qemu-system-x86_64 not found".to_string());
    }
    let disk = abs_path_from_rel_or_abs(&vm.disk_path);
    if !disk.exists() {
        return Err(format!("disk image not found: {}", disk.display()));
    }

    let runtime_dir = vm_runtime_dir(managed_root_dir);
    fs::create_dir_all(&runtime_dir).map_err(|e| format!("create runtime dir failed: {e}"))?;
    let pid_path = vm_pid_path(managed_root_dir, &vm.name);

    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.arg("-name")
        .arg(&vm.name)
        .arg("-daemonize")
        .arg("-display")
        .arg("none")
        .arg("-no-reboot")
        .arg("-pidfile")
        .arg(&pid_path)
        .arg("-m")
        .arg(vm.memory_mb.to_string())
        .arg("-smp")
        .arg(vm.cpu.to_string())
        .arg("-drive")
        .arg(format!("file={},if=virtio,format=qcow2", disk.display()))
        .arg("-netdev")
        .arg(format!("user,id=net0,hostfwd=tcp:127.0.0.1:{ssh_port}-:22"))
        .arg("-device")
        .arg("virtio-net-pci,netdev=net0");

    if let Some(iso) = resolve_iso_path(&vm.os_image)? {
        cmd.arg("-cdrom").arg(iso);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("qemu launch failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("qemu failed: {}", if stderr.is_empty() { "unknown" } else { &stderr }));
    }

    let pid = read_pid_file(&pid_path).ok_or_else(|| "qemu pidfile not generated".to_string())?;
    if !pid_alive(pid) {
        return Err(format!("qemu pid {pid} is not alive after launch"));
    }
    Ok((
        pid,
        format!("qemu started with pid {pid}, ssh forwarded on 127.0.0.1:{ssh_port}"),
    ))
}

fn qemu_snapshot_create(vm: &VmInstance, snapshot: &str) -> Result<String, String> {
    if !command_available("qemu-img") {
        return Err("qemu-img not found".to_string());
    }
    let disk = abs_path_from_rel_or_abs(&vm.disk_path);
    let output = Command::new("qemu-img")
        .args(["snapshot", "-c", snapshot])
        .arg(&disk)
        .output()
        .map_err(|e| format!("qemu-img snapshot create failed: {e}"))?;
    if output.status.success() {
        return Ok(format!("snapshot created: {snapshot}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!(
        "qemu-img snapshot create error: {}",
        if stderr.is_empty() { "unknown" } else { &stderr }
    ))
}

fn qemu_snapshot_apply(vm: &VmInstance, snapshot: &str) -> Result<String, String> {
    if !command_available("qemu-img") {
        return Err("qemu-img not found".to_string());
    }
    let disk = abs_path_from_rel_or_abs(&vm.disk_path);
    let output = Command::new("qemu-img")
        .args(["snapshot", "-a", snapshot])
        .arg(&disk)
        .output()
        .map_err(|e| format!("qemu-img snapshot apply failed: {e}"))?;
    if output.status.success() {
        return Ok(format!("snapshot applied: {snapshot}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!(
        "qemu-img snapshot apply error: {}",
        if stderr.is_empty() { "unknown" } else { &stderr }
    ))
}

fn qemu_snapshot_delete(vm: &VmInstance, snapshot: &str) -> Result<String, String> {
    if !command_available("qemu-img") {
        return Err("qemu-img not found".to_string());
    }
    let disk = abs_path_from_rel_or_abs(&vm.disk_path);
    let output = Command::new("qemu-img")
        .args(["snapshot", "-d", snapshot])
        .arg(&disk)
        .output()
        .map_err(|e| format!("qemu-img snapshot delete failed: {e}"))?;
    if output.status.success() {
        return Ok(format!("snapshot deleted: {snapshot}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!(
        "qemu-img snapshot delete error: {}",
        if stderr.is_empty() { "unknown" } else { &stderr }
    ))
}

fn qemu_snapshot_list(vm: &VmInstance) -> Result<Vec<VmSnapshotEntry>, String> {
    if !command_available("qemu-img") {
        return Err("qemu-img not found".to_string());
    }
    let disk = abs_path_from_rel_or_abs(&vm.disk_path);
    let output = Command::new("qemu-img")
        .args(["snapshot", "-l"])
        .arg(&disk)
        .output()
        .map_err(|e| format!("qemu-img snapshot list failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "qemu-img snapshot list error: {}",
            if stderr.is_empty() { "unknown" } else { &stderr }
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out: Vec<VmSnapshotEntry> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("Snapshot list")
            || trimmed.starts_with("ID")
            || trimmed.starts_with("---")
        {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.len() >= 2 {
            let tag = cols[1].to_string();
            let vm_size = if cols.len() >= 3 {
                cols[2].to_string()
            } else {
                String::new()
            };
            let created_at = if cols.len() >= 5 {
                format!("{} {}", cols[3], cols[4])
            } else {
                String::new()
            };
            let vm_clock = if cols.len() >= 6 {
                cols[5..].join(" ")
            } else {
                String::new()
            };
            out.push(VmSnapshotEntry {
                tag,
                vm_size,
                created_at,
                vm_clock,
            });
        }
    }
    Ok(out)
}

fn qemu_clone_disk_from_snapshot(
    source_vm: &VmInstance,
    snapshot: Option<&str>,
    output_path: &Path,
) -> Result<String, String> {
    if !command_available("qemu-img") {
        return Err("qemu-img not found".to_string());
    }
    let src = abs_path_from_rel_or_abs(&source_vm.disk_path);
    if !src.exists() {
        return Err(format!("source disk not found: {}", src.display()));
    }
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create output dir failed: {e}"))?;
    }
    let mut cmd = Command::new("qemu-img");
    cmd.args(["convert", "-f", "qcow2", "-O", "qcow2"]);
    if let Some(snap) = snapshot {
        cmd.args(["-l", snap]);
    }
    cmd.arg(&src).arg(output_path);
    let out = cmd
        .output()
        .map_err(|e| format!("qemu-img convert failed: {e}"))?;
    if out.status.success() {
        if let Some(snap) = snapshot {
            Ok(format!("vm cloned from snapshot {snap}"))
        } else {
            Ok("vm cloned from current disk".to_string())
        }
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!(
            "qemu-img convert error: {}",
            if stderr.is_empty() { "unknown" } else { &stderr }
        ))
    }
}

fn run_ssh_command(
    ssh_user: &str,
    ssh_port: u16,
    command_text: &str,
    timeout_sec: u64,
) -> Result<VmExecResponse, String> {
    run_ssh_command_controlled(ssh_user, ssh_port, command_text, timeout_sec, None, None)
}

fn run_ssh_command_controlled(
    ssh_user: &str,
    ssh_port: u16,
    command_text: &str,
    timeout_sec: u64,
    pid_file: Option<&Path>,
    cancel_file: Option<&Path>,
) -> Result<VmExecResponse, String> {
    if !command_available("ssh") {
        return Err("ssh command not found".to_string());
    }
    let timeout = timeout_sec.clamp(1, 3600);
    let mut child = Command::new("ssh")
        .args([
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "ConnectTimeout=5",
            "-p",
            &ssh_port.to_string(),
            &format!("{}@127.0.0.1", ssh_user),
            "--",
            command_text,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn ssh failed: {e}"))?;

    if let Some(p) = pid_file {
        let _ = fs::write(p, format!("{}", child.id()));
    }
    if let Some(c) = cancel_file {
        let _ = fs::remove_file(c);
    }

    let started = Instant::now();
    loop {
        let status = child
            .wait_timeout(Duration::from_millis(200))
            .map_err(|e| format!("wait ssh failed: {e}"))?;
        if let Some(_s) = status {
            break;
        }
        if started.elapsed() >= Duration::from_secs(timeout) {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(p) = pid_file {
                let _ = fs::remove_file(p);
            }
            return Ok(VmExecResponse {
                ok: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: String::new(),
                message: format!("ssh command timed out after {timeout}s"),
            });
        }
        if let Some(c) = cancel_file {
            if c.exists() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(c);
                if let Some(p) = pid_file {
                    let _ = fs::remove_file(p);
                }
                return Ok(VmExecResponse {
                    ok: false,
                    exit_code: -2,
                    stdout: String::new(),
                    stderr: String::new(),
                    message: "vm exec canceled".to_string(),
                });
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("collect ssh output failed: {e}"))?;
    if let Some(p) = pid_file {
        let _ = fs::remove_file(p);
    }
    if let Some(c) = cancel_file {
        let _ = fs::remove_file(c);
    }
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok(VmExecResponse {
        ok: output.status.success(),
        exit_code: code,
        stdout,
        stderr,
        message: if output.status.success() {
            "vm exec succeeded".to_string()
        } else {
            format!("vm exec failed with exit code {code}")
        },
    })
}

fn wait_until_ssh_ready(ssh_user: &str, ssh_port: u16, wait_ready_sec: u64) -> Result<(), String> {
    if wait_ready_sec == 0 {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(wait_ready_sec.clamp(1, 600));
    loop {
        match run_ssh_command(ssh_user, ssh_port, "echo VM_READY", 6) {
            Ok(resp) if resp.ok => return Ok(()),
            Ok(_) | Err(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "vm ssh is not ready within {}s",
                wait_ready_sec.clamp(1, 600)
            ));
        }
        thread::sleep(Duration::from_millis(1000));
    }
}

fn stop_qemu_process(managed_root_dir: &str, vm: &VmInstance) -> Result<String, String> {
    let pid_path = vm_pid_path(managed_root_dir, &vm.name);
    let pid = vm.process_id.or_else(|| read_pid_file(&pid_path));
    let Some(pid) = pid else {
        let _ = fs::remove_file(pid_path);
        return Ok("qemu pid missing, treated as stopped".to_string());
    };

    if !pid_alive(pid) {
        let _ = fs::remove_file(pid_path);
        return Ok(format!("qemu pid {pid} already stopped"));
    }

    if stop_pid(pid) {
        let _ = fs::remove_file(pid_path);
        Ok(format!("qemu pid {pid} stopped"))
    } else {
        Err(format!("failed to stop qemu pid {pid}"))
    }
}

fn reconcile_vm_power_state(managed_root_dir: &str, vm: &mut VmInstance) -> bool {
    if vm.backend != "qemu" {
        return false;
    }
    let pid_path = vm_pid_path(managed_root_dir, &vm.name);
    let live_pid = vm.process_id.or_else(|| read_pid_file(&pid_path));
    let was = vm.power_state.clone();

    match live_pid {
        Some(pid) if pid_alive(pid) => {
            vm.process_id = Some(pid);
            vm.power_state = "running".to_string();
            if vm.last_message.is_empty() {
                vm.last_message = format!("qemu running pid {pid}");
            }
        }
        Some(pid) => {
            vm.process_id = None;
            vm.power_state = "stopped".to_string();
            vm.last_message = format!("qemu pid {pid} is not alive");
            let _ = fs::remove_file(pid_path);
        }
        None => {
            vm.process_id = None;
            vm.power_state = "stopped".to_string();
        }
    }

    if vm.power_state != was {
        vm.updated_at_unix = now_unix();
        return true;
    }
    false
}

pub(crate) async fn get_vm_status(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let caps = probe_capabilities();
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);

    let mut changed = false;
    for vm in state.vms.values_mut() {
        if reconcile_vm_power_state(&ctx.managed_root_dir, vm) {
            changed = true;
        }
    }
    if changed && ctx.can_write {
        let _ = save_vm_state(&ctx.managed_root_dir, &state);
    }

    let mut list: Vec<VmInstance> = state.vms.values().cloned().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    HttpResponse::Ok().json(VmStatusResponse {
        readonly: !ctx.can_write,
        capabilities: caps,
        vms: list,
    })
}

pub(crate) async fn get_vm_logs(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmLogQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let tail = query.tail.unwrap_or(8000).clamp(200, 200_000);
    let path = vm_log_path(&ctx.managed_root_dir, &name);
    let raw = fs::read_to_string(&path).unwrap_or_default();
    let text = tail_text(&raw, tail);
    HttpResponse::Ok().json(VmLogsResponse {
        name,
        path: path
            .strip_prefix(workspace_root())
            .unwrap_or(&path)
            .display()
            .to_string(),
        bytes: raw.len(),
        text,
    })
}

pub(crate) async fn list_vm_snapshots(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmSnapshotListQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.get(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    let snapshots = if vm.backend == "qemu" {
        match qemu_snapshot_list(vm) {
            Ok(v) => v,
            Err(e) => return HttpResponse::InternalServerError().body(e),
        }
    } else {
        Vec::new()
    };

    HttpResponse::Ok().json(VmSnapshotListResponse { name, snapshots })
}

pub(crate) async fn create_vm_snapshot(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSnapshotPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let snapshot_name = match sanitize_snapshot_name(&body.snapshot) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid snapshot name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    reconcile_vm_power_state(&ctx.managed_root_dir, vm);
    if vm.backend != "qemu" {
        return HttpResponse::BadRequest().body("snapshot is only available for qemu backend");
    }
    if vm.power_state == "running" {
        return HttpResponse::BadRequest().body("stop vm before creating snapshot");
    }

    let msg = match qemu_snapshot_create(vm, &snapshot_name) {
        Ok(v) => v,
        Err(e) => {
            vm.last_message = e.clone();
            append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("snapshot create failed: {e}"));
            if let Err(se) = save_vm_state(&ctx.managed_root_dir, &state) {
                return HttpResponse::InternalServerError().body(format!("save vm state failed: {se}"));
            }
            return HttpResponse::InternalServerError().body(e);
        }
    };

    vm.updated_at_unix = now_unix();
    vm.last_message = msg.clone();
    append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("snapshot create: {snapshot_name}"));
    let snapshot = vm.clone();
    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }
    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective: true,
        message: msg,
        vm: Some(snapshot),
    })
}

pub(crate) async fn apply_vm_snapshot(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSnapshotPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let snapshot_name = match sanitize_snapshot_name(&body.snapshot) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid snapshot name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    reconcile_vm_power_state(&ctx.managed_root_dir, vm);
    if vm.backend != "qemu" {
        return HttpResponse::BadRequest().body("snapshot is only available for qemu backend");
    }
    if vm.power_state == "running" {
        return HttpResponse::BadRequest().body("stop vm before applying snapshot");
    }

    let msg = match qemu_snapshot_apply(vm, &snapshot_name) {
        Ok(v) => v,
        Err(e) => {
            vm.last_message = e.clone();
            append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("snapshot apply failed: {e}"));
            if let Err(se) = save_vm_state(&ctx.managed_root_dir, &state) {
                return HttpResponse::InternalServerError().body(format!("save vm state failed: {se}"));
            }
            return HttpResponse::InternalServerError().body(e);
        }
    };

    vm.updated_at_unix = now_unix();
    vm.last_message = msg.clone();
    append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("snapshot apply: {snapshot_name}"));
    let snapshot = vm.clone();
    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }
    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective: true,
        message: msg,
        vm: Some(snapshot),
    })
}

pub(crate) async fn delete_vm_snapshot(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSnapshotPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let snapshot_name = match sanitize_snapshot_name(&body.snapshot) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid snapshot name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    reconcile_vm_power_state(&ctx.managed_root_dir, vm);
    if vm.backend != "qemu" {
        return HttpResponse::BadRequest().body("snapshot is only available for qemu backend");
    }
    if vm.power_state == "running" {
        return HttpResponse::BadRequest().body("stop vm before deleting snapshot");
    }

    let msg = match qemu_snapshot_delete(vm, &snapshot_name) {
        Ok(v) => v,
        Err(e) => {
            vm.last_message = e.clone();
            append_vm_log(
                &ctx.managed_root_dir,
                &vm.name,
                &format!("snapshot delete failed: {e}"),
            );
            if let Err(se) = save_vm_state(&ctx.managed_root_dir, &state) {
                return HttpResponse::InternalServerError().body(format!("save vm state failed: {se}"));
            }
            return HttpResponse::InternalServerError().body(e);
        }
    };

    vm.updated_at_unix = now_unix();
    vm.last_message = msg.clone();
    append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("snapshot delete: {snapshot_name}"));
    let snapshot = vm.clone();
    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }
    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective: true,
        message: msg,
        vm: Some(snapshot),
    })
}

pub(crate) async fn clone_vm_from_snapshot(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmClonePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let source_name = match sanitize_vm_name(&body.source_name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid source vm name"),
    };
    let new_name = match sanitize_vm_name(&body.new_name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid new vm name"),
    };
    if source_name == new_name {
        return HttpResponse::BadRequest().body("source vm and new vm must be different");
    }
    let snapshot_name = if body.snapshot.trim().is_empty() {
        None
    } else {
        match sanitize_snapshot_name(&body.snapshot) {
            Some(v) => Some(v),
            None => return HttpResponse::BadRequest().body("invalid snapshot name"),
        }
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    if state.vms.contains_key(&new_name) {
        return HttpResponse::Conflict().body("new vm already exists");
    }
    let source_vm = {
        let Some(vm) = state.vms.get_mut(&source_name) else {
            return HttpResponse::NotFound().body("source vm not found");
        };
        reconcile_vm_power_state(&ctx.managed_root_dir, vm);
        if vm.backend != "qemu" {
            return HttpResponse::BadRequest().body("clone is only available for qemu backend");
        }
        if vm.power_state == "running" {
            return HttpResponse::BadRequest().body("stop source vm before cloning");
        }
        vm.clone()
    };

    if let Some(ref snap) = snapshot_name {
        let list = match qemu_snapshot_list(&source_vm) {
            Ok(v) => v,
            Err(e) => return HttpResponse::InternalServerError().body(e),
        };
        if !list.iter().any(|x| x.tag == *snap) {
            return HttpResponse::BadRequest().body("snapshot not found on source vm");
        }
    }

    let disk_dir = vm_disk_dir(&ctx.managed_root_dir);
    if let Err(e) = fs::create_dir_all(&disk_dir) {
        return HttpResponse::InternalServerError().body(format!("create vm disk dir failed: {e}"));
    }
    let new_disk_path = disk_dir.join(format!("{new_name}.qcow2"));
    if new_disk_path.exists() {
        return HttpResponse::Conflict().body("new vm disk file already exists");
    }

    let clone_msg = match qemu_clone_disk_from_snapshot(
        &source_vm,
        snapshot_name.as_deref(),
        &new_disk_path,
    ) {
        Ok(v) => v,
        Err(e) => return HttpResponse::InternalServerError().body(e),
    };

    let now = now_unix();
    let instance = VmInstance {
        name: new_name.clone(),
        backend: "qemu".to_string(),
        power_state: "stopped".to_string(),
        cpu: source_vm.cpu,
        memory_mb: source_vm.memory_mb,
        disk_gb: source_vm.disk_gb,
        disk_path: new_disk_path
            .strip_prefix(workspace_root())
            .unwrap_or(&new_disk_path)
            .display()
            .to_string(),
        os_image: source_vm.os_image.clone(),
        created_at_unix: now,
        updated_at_unix: now,
        last_message: clone_msg.clone(),
        process_id: None,
        ssh_port: None,
        ssh_user: source_vm.ssh_user.clone(),
    };
    state.vms.insert(new_name.clone(), instance.clone());
    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }

    append_vm_log(
        &ctx.managed_root_dir,
        &source_name,
        &format!(
            "clone created new_vm={} snapshot={}",
            new_name,
            snapshot_name.clone().unwrap_or_else(|| "-".to_string())
        ),
    );
    append_vm_log(
        &ctx.managed_root_dir,
        &new_name,
        &format!(
            "created by clone from source_vm={} snapshot={} ({})",
            source_name,
            snapshot_name.unwrap_or_else(|| "-".to_string()),
            clone_msg
        ),
    );

    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective: true,
        message: "vm clone created".to_string(),
        vm: Some(instance),
    })
}

pub(crate) async fn exec_in_vm(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmExecPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let cmd = body.command.trim();
    if cmd.is_empty() {
        return HttpResponse::BadRequest().body("command is empty");
    }
    let timeout = if body.timeout_sec == 0 {
        30
    } else {
        body.timeout_sec.clamp(1, 3600)
    };
    let wait_ready_sec = body.wait_ready_sec.clamp(0, 600);

    // Read connection info under lock, then release lock before potentially long-running ssh calls.
    let (ssh_user, ssh_port) = {
        let _guard = lock_recover(&data.projects_lock, "projects_lock");
        let mut state = load_vm_state(&ctx.managed_root_dir);
        let Some(vm) = state.vms.get_mut(&name) else {
            return HttpResponse::NotFound().body("vm not found");
        };
        reconcile_vm_power_state(&ctx.managed_root_dir, vm);
        if vm.backend != "qemu" {
            return HttpResponse::BadRequest().body("vm exec is only available for qemu backend");
        }
        if vm.power_state != "running" {
            return HttpResponse::BadRequest().body("vm is not running");
        }
        let ssh_port = vm.ssh_port.unwrap_or(0);
        if ssh_port == 0 {
            return HttpResponse::BadRequest().body("vm ssh port is not set");
        }
        (normalize_ssh_user(&vm.ssh_user), ssh_port)
    };

    if let Err(e) = wait_until_ssh_ready(&ssh_user, ssh_port, wait_ready_sec) {
        let _guard = lock_recover(&data.projects_lock, "projects_lock");
        let mut state = load_vm_state(&ctx.managed_root_dir);
        if let Some(vm) = state.vms.get_mut(&name) {
            vm.last_message = e.clone();
            vm.updated_at_unix = now_unix();
            append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("exec blocked: {e}"));
            let _ = save_vm_state(&ctx.managed_root_dir, &state);
        }
        return HttpResponse::Ok().json(VmExecResponse {
            ok: false,
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            message: e,
        });
    }

    let pid_path = vm_exec_pid_path(&ctx.managed_root_dir, &name);
    let cancel_path = vm_exec_cancel_path(&ctx.managed_root_dir, &name);
    let resp = match run_ssh_command_controlled(
        &ssh_user,
        ssh_port,
        cmd,
        timeout,
        Some(&pid_path),
        Some(&cancel_path),
    ) {
        Ok(v) => v,
        Err(e) => {
            let _guard = lock_recover(&data.projects_lock, "projects_lock");
            let mut state = load_vm_state(&ctx.managed_root_dir);
            if let Some(vm) = state.vms.get_mut(&name) {
                append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("exec failed: {e}"));
                vm.last_message = e.clone();
                vm.updated_at_unix = now_unix();
                let _ = save_vm_state(&ctx.managed_root_dir, &state);
            }
            return HttpResponse::InternalServerError().body(e);
        }
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    if let Some(vm) = state.vms.get_mut(&name) {
        vm.last_message = resp.message.clone();
        vm.updated_at_unix = now_unix();
        append_vm_log(
            &ctx.managed_root_dir,
            &vm.name,
            &format!("exec cmd={} exit={} ok={}", cmd, resp.exit_code, resp.ok),
        );
        let _ = save_vm_state(&ctx.managed_root_dir, &state);
    }
    HttpResponse::Ok().json(resp)
}

pub(crate) async fn cancel_vm_exec(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmExecCancelPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };

    let cancel_path = vm_exec_cancel_path(&ctx.managed_root_dir, &name);
    let _ = fs::write(&cancel_path, b"1");
    let pid_path = vm_exec_pid_path(&ctx.managed_root_dir, &name);
    if let Some(pid) = read_pid_file(&pid_path) {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    append_vm_log(&ctx.managed_root_dir, &name, "exec cancel requested");
    HttpResponse::Ok().json(VmExecCancelResponse {
        ok: true,
        message: "cancel signal sent".to_string(),
    })
}

pub(crate) async fn bootstrap_vm(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmBootstrapPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _profile = if body.profile.trim().is_empty() {
        "dev-basic".to_string()
    } else {
        body.profile.trim().to_string()
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };
    reconcile_vm_power_state(&ctx.managed_root_dir, vm);
    if vm.backend != "qemu" {
        return HttpResponse::BadRequest().body("bootstrap is only available for qemu backend");
    }
    if vm.power_state != "running" {
        return HttpResponse::BadRequest().body("vm is not running");
    }
    let ssh_port = vm.ssh_port.unwrap_or(0);
    if ssh_port == 0 {
        return HttpResponse::BadRequest().body("vm ssh port is not set");
    }
    let ssh_user = normalize_ssh_user(&vm.ssh_user);
    drop(_guard);

    let script = r#"sh -lc 'set -e
if command -v apt-get >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y git curl build-essential python3 python3-pip nodejs npm
elif command -v dnf >/dev/null 2>&1; then
  dnf install -y git curl gcc gcc-c++ make python3 python3-pip nodejs npm
elif command -v yum >/dev/null 2>&1; then
  yum install -y git curl gcc gcc-c++ make python3 python3-pip nodejs npm
elif command -v apk >/dev/null 2>&1; then
  apk add --no-cache git curl build-base python3 py3-pip nodejs npm
else
  echo "No supported package manager found"; exit 2
fi
echo "BOOTSTRAP_OK"
'"#;
    let resp = match run_ssh_command(&ssh_user, ssh_port, script, 1800) {
        Ok(v) => v,
        Err(e) => {
            append_vm_log(&ctx.managed_root_dir, &name, &format!("bootstrap failed: {e}"));
            return HttpResponse::InternalServerError().body(e);
        }
    };
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("bootstrap finished ok={} exit={}", resp.ok, resp.exit_code),
    );
    HttpResponse::Ok().json(resp)
}

pub(crate) async fn list_vm_exec_queue(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmQueueQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let items = load_exec_queue(&ctx.managed_root_dir, &name);
    HttpResponse::Ok().json(VmQueueResponse { name, items })
}

pub(crate) async fn vm_exec_queue_stats(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmQueueQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let state = load_vm_state(&ctx.managed_root_dir);
    let items = load_exec_queue(&ctx.managed_root_dir, &name);
    let total = items.len();
    let pending = items.iter().filter(|x| x.status == "pending").count();
    let running = items.iter().filter(|x| x.status == "running").count();
    let done = items.iter().filter(|x| x.status == "done").count();
    let failed = items.iter().filter(|x| x.status == "failed").count();
    let canceled = items.iter().filter(|x| x.status == "canceled").count();
    let finished = done + failed + canceled;
    let done_success_rate = if finished == 0 {
        0.0
    } else {
        (done as f64) * 100.0 / (finished as f64)
    };
    let durations: Vec<f64> = items
        .iter()
        .filter(|x| x.started_at_unix > 0 && x.finished_at_unix >= x.started_at_unix)
        .map(|x| (x.finished_at_unix - x.started_at_unix) as f64)
        .collect();
    let avg_duration_sec = if durations.is_empty() {
        0.0
    } else {
        durations.iter().sum::<f64>() / durations.len() as f64
    };
    let running_total_all_vms = count_running_exec_tasks_all_vms(&ctx.managed_root_dir, &state);
    let running_limit_all_vms = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    let (watchdog_recovered_total, watchdog_last_recovered_unix) =
        queue_watchdog_recovery_stats(&items);
    let scheduler = load_vm_policy_config(&ctx.managed_root_dir).scheduler;
    let now = now_unix();
    let oldest_pending_age_sec = items
        .iter()
        .filter(|x| x.status == "pending" && now >= x.next_run_after_unix)
        .map(|x| now.saturating_sub(x.created_at_unix))
        .max()
        .unwrap_or(0);
    let top_pending_effective_priority = items
        .iter()
        .filter(|x| x.status == "pending" && now >= x.next_run_after_unix)
        .map(|x| {
            effective_priority_with_aging(
                x,
                now,
                scheduler.priority_aging_step_sec,
                scheduler.priority_aging_max_boost,
            )
        })
        .max()
        .unwrap_or(-100);
    HttpResponse::Ok().json(VmQueueStatsResponse {
        name,
        total,
        pending,
        running,
        done,
        failed,
        canceled,
        done_success_rate,
        avg_duration_sec,
        running_total_all_vms,
        running_limit_all_vms,
        watchdog_recovered_total,
        watchdog_last_recovered_unix,
        oldest_pending_age_sec,
        top_pending_effective_priority,
    })
}

pub(crate) async fn enqueue_vm_exec(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmExecEnqueuePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let cmd = body.command.trim();
    if cmd.is_empty() {
        return HttpResponse::BadRequest().body("command is empty");
    }
    let timeout_sec = if body.timeout_sec == 0 {
        30
    } else {
        body.timeout_sec.clamp(1, 3600)
    };
    let wait_ready_sec = body.wait_ready_sec.clamp(0, 600);
    let priority = body.priority.clamp(-100, 100);
    let retry_max = body.retry_max.clamp(0, 10);

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let state = load_vm_state(&ctx.managed_root_dir);
    if !state.vms.contains_key(&name) {
        return HttpResponse::NotFound().body("vm not found");
    }
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    if queue_has_active_duplicate(&items, cmd) {
        return HttpResponse::Conflict().body("duplicate active command in queue");
    }
    let queue_limit = vm_queue_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    if let Err(e) = ensure_queue_capacity(items.len(), 1, queue_limit) {
        return HttpResponse::TooManyRequests().body(e);
    }
    items.push(queue_item_from_values(
        cmd,
        timeout_sec,
        wait_ready_sec,
        priority,
        retry_max,
    ));
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    drop(_guard);
    append_vm_log(&ctx.managed_root_dir, &name, "exec task enqueued");
    let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    let _ = dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
    HttpResponse::Ok().json(VmQueueResponse { name, items })
}

pub(crate) async fn enqueue_vm_exec_batch(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmExecEnqueueBatchPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    if body.tasks.is_empty() {
        return HttpResponse::BadRequest().body("tasks is empty");
    }
    let default_timeout_sec = if body.timeout_sec == 0 {
        30
    } else {
        body.timeout_sec.clamp(1, 3600)
    };
    let default_wait_ready_sec = body.wait_ready_sec.clamp(0, 600);
    let default_priority = body.priority.clamp(-100, 100);
    let default_retry_max = body.retry_max.clamp(0, 10);

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let state = load_vm_state(&ctx.managed_root_dir);
    if !state.vms.contains_key(&name) {
        return HttpResponse::NotFound().body("vm not found");
    }
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut new_items: Vec<VmExecQueueItem> = Vec::new();
    let mut seen_commands: BTreeSet<String> = BTreeSet::new();
    for task in &body.tasks {
        if let Some(item) = normalize_batch_task(
            task,
            default_timeout_sec,
            default_wait_ready_sec,
            default_priority,
            default_retry_max,
        ) {
            let key = item.command.trim().to_string();
            if key.is_empty() {
                continue;
            }
            if queue_has_active_duplicate(&items, &key) {
                continue;
            }
            if !seen_commands.insert(key) {
                continue;
            }
            new_items.push(item);
        }
    }
    let added = new_items.len();
    if added == 0 {
        return HttpResponse::Conflict().body("all tasks are duplicate active commands");
    }
    let queue_limit = vm_queue_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    if let Err(e) = ensure_queue_capacity(items.len(), added, queue_limit) {
        return HttpResponse::TooManyRequests().body(e);
    }
    items.extend(new_items);
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    drop(_guard);
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("exec batch enqueued tasks={added}"),
    );
    let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    let _ = dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
    HttpResponse::Ok().json(VmQueueResponse { name, items })
}

pub(crate) async fn list_vm_exec_profiles(
    req: HttpRequest,
    data: web::Data<AppState>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    HttpResponse::Ok().json(VmExecProfilesResponse {
        profiles: list_all_profiles(&ctx.managed_root_dir),
    })
}

pub(crate) async fn get_vm_exec_profile_detail(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmExecProfileDetailQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let profile = query.profile.trim();
    if profile.is_empty() {
        return HttpResponse::BadRequest().body("profile is empty");
    }
    if let Some(builtin) = build_builtin_profile_batch_tasks(profile, &ProfileRuntimeOptions::default()) {
        return HttpResponse::Ok().json(VmExecProfileDetailResponse {
            profile: profile.to_string(),
            commands: builtin.into_iter().map(|x| x.command).collect(),
        });
    }
    let store = load_custom_profiles(&ctx.managed_root_dir);
    let Some(commands) = store.profiles.get(profile) else {
        return HttpResponse::NotFound().body("profile not found");
    };
    HttpResponse::Ok().json(VmExecProfileDetailResponse {
        profile: profile.to_string(),
        commands: commands.clone(),
    })
}

pub(crate) async fn save_vm_exec_custom_profile(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmExecCustomProfileSavePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let Some(name) = sanitize_custom_profile_name(&body.name) else {
        return HttpResponse::BadRequest()
            .body("invalid profile name, use custom-<lowercase-and-dash>");
    };
    let commands = normalize_custom_commands(&body.commands);
    if commands.is_empty() {
        return HttpResponse::BadRequest().body("commands is empty");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut store = load_custom_profiles(&ctx.managed_root_dir);
    store.profiles.insert(name.clone(), commands);
    if let Err(e) = save_custom_profiles(&ctx.managed_root_dir, &store) {
        return HttpResponse::InternalServerError().body(format!("save custom profiles failed: {e}"));
    }
    HttpResponse::Ok().json(VmExecProfilesResponse {
        profiles: list_all_profiles(&ctx.managed_root_dir),
    })
}

pub(crate) async fn delete_vm_exec_custom_profile(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmExecCustomProfileDeletePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let Some(name) = sanitize_custom_profile_name(&body.name) else {
        return HttpResponse::BadRequest()
            .body("invalid profile name, use custom-<lowercase-and-dash>");
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut store = load_custom_profiles(&ctx.managed_root_dir);
    if store.profiles.remove(&name).is_none() {
        return HttpResponse::NotFound().body("custom profile not found");
    }
    if let Err(e) = save_custom_profiles(&ctx.managed_root_dir, &store) {
        return HttpResponse::InternalServerError().body(format!("save custom profiles failed: {e}"));
    }
    HttpResponse::Ok().json(VmExecProfilesResponse {
        profiles: list_all_profiles(&ctx.managed_root_dir),
    })
}

pub(crate) async fn preview_vm_exec_profile(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmExecProfilePreviewQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let profile = query.profile.trim();
    if profile.is_empty() {
        return HttpResponse::BadRequest().body("profile is empty");
    }
    let opts = match parse_profile_options(&query.workdir, &query.test_cmd) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let Some(tasks) = build_profile_batch_tasks(&ctx.managed_root_dir, profile, &opts) else {
        return HttpResponse::BadRequest().body("unknown profile");
    };
    HttpResponse::Ok().json(VmExecProfilePreviewResponse {
        profile: profile.to_string(),
        tasks: tasks.into_iter().map(|x| x.command).collect(),
    })
}

pub(crate) async fn enqueue_vm_exec_profile(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmExecEnqueueProfilePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let profile = body.profile.trim();
    if profile.is_empty() {
        return HttpResponse::BadRequest().body("profile is empty");
    }
    let opts = match parse_profile_options(&body.workdir, &body.test_cmd) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let Some(tasks) = build_profile_batch_tasks(&ctx.managed_root_dir, profile, &opts) else {
        return HttpResponse::BadRequest().body("unknown profile");
    };
    let default_timeout_sec = if body.timeout_sec == 0 {
        30
    } else {
        body.timeout_sec.clamp(1, 3600)
    };
    let default_wait_ready_sec = body.wait_ready_sec.clamp(0, 600);
    let default_priority = body.priority.clamp(-100, 100);
    let default_retry_max = body.retry_max.clamp(0, 10);

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let state = load_vm_state(&ctx.managed_root_dir);
    if !state.vms.contains_key(&name) {
        return HttpResponse::NotFound().body("vm not found");
    }
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut new_items: Vec<VmExecQueueItem> = Vec::new();
    let mut seen_commands: BTreeSet<String> = BTreeSet::new();
    for task in &tasks {
        if let Some(item) = normalize_batch_task(
            task,
            default_timeout_sec,
            default_wait_ready_sec,
            default_priority,
            default_retry_max,
        ) {
            let key = item.command.trim().to_string();
            if key.is_empty() {
                continue;
            }
            if queue_has_active_duplicate(&items, &key) {
                continue;
            }
            if !seen_commands.insert(key) {
                continue;
            }
            new_items.push(item);
        }
    }
    let added = new_items.len();
    if added == 0 {
        return HttpResponse::Conflict().body("profile has no new command to enqueue");
    }
    let queue_limit = vm_queue_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    if let Err(e) = ensure_queue_capacity(items.len(), added, queue_limit) {
        return HttpResponse::TooManyRequests().body(e);
    }
    items.extend(new_items);
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    drop(_guard);
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("exec profile enqueued profile={profile} tasks={added}"),
    );
    let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    let _ = dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
    HttpResponse::Ok().json(VmQueueResponse { name, items })
}

pub(crate) async fn preview_vm_self_debug_plan(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugPlanPayload>,
) -> impl Responder {
    let _ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let mut tasks = match build_self_debug_tasks(&body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let budget = body.max_task_budget.clamp(0, 500) as usize;
    if budget > 0 && tasks.len() > budget {
        tasks.truncate(budget);
    }
    HttpResponse::Ok().json(VmSelfDebugPlanResponse {
        name,
        total_tasks: tasks.len(),
        tasks: tasks.into_iter().map(|x| x.command).collect(),
        message: "self-debug plan generated".to_string(),
        run_id: String::new(),
    })
}

pub(crate) async fn start_vm_self_debug_plan(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugPlanPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let mut tasks = match build_self_debug_tasks(&body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let budget = body.max_task_budget.clamp(0, 500) as usize;
    if budget > 0 && tasks.len() > budget {
        tasks.truncate(budget);
    }
    let run_id = if body.run_id.trim().is_empty() {
        self_debug_run_id()
    } else {
        match sanitize_self_debug_run_id(&body.run_id) {
            Some(v) => v,
            None => return HttpResponse::BadRequest().body("invalid run_id"),
        }
    };
    let default_timeout_sec = if body.timeout_sec == 0 {
        120
    } else {
        body.timeout_sec.clamp(1, 3600)
    };
    let default_wait_ready_sec = body.wait_ready_sec.clamp(0, 600);
    let default_priority = body.priority.clamp(-100, 100);
    let default_retry_max = body.retry_max.clamp(0, 10);
    let run_max_runtime_sec = body.max_runtime_sec.clamp(0, 86_400);

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let state = load_vm_state(&ctx.managed_root_dir);
    if !state.vms.contains_key(&name) {
        return HttpResponse::NotFound().body("vm not found");
    }
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut new_items: Vec<VmExecQueueItem> = Vec::new();
    for task in &tasks {
        if let Some(mut item) = normalize_batch_task(
            task,
            default_timeout_sec,
            default_wait_ready_sec,
            default_priority,
            default_retry_max,
        ) {
            item.run_id = run_id.clone();
            item.run_kind = "self_debug".to_string();
            item.run_max_runtime_sec = run_max_runtime_sec;
            new_items.push(item);
        }
    }
    let added = new_items.len();
    if added == 0 {
        return HttpResponse::BadRequest().body("self-debug plan has no runnable command");
    }
    let queue_limit = vm_queue_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    if let Err(e) = ensure_queue_capacity(items.len(), added, queue_limit) {
        return HttpResponse::TooManyRequests().body(e);
    }
    items.extend(new_items);
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    drop(_guard);
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!(
            "self-debug plan enqueued run_id={} tasks={} mode={}",
            run_id,
            added,
            if body.run_id.trim().is_empty() {
                "new"
            } else {
                "append"
            }
        ),
    );
    let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    let _ = dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
    HttpResponse::Ok().json(VmSelfDebugPlanResponse {
        name,
        total_tasks: tasks.len(),
        tasks: tasks.into_iter().map(|x| x.command).collect(),
        message: "self-debug plan enqueued".to_string(),
        run_id,
    })
}

pub(crate) async fn list_vm_self_debug_runs(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmSelfDebugRunsQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let items = load_exec_queue(&ctx.managed_root_dir, &name);
    let runs = collect_self_debug_runs(&items);
    HttpResponse::Ok().json(VmSelfDebugRunsResponse { name, runs })
}

pub(crate) async fn list_vm_self_debug_history(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmSelfDebugRunsQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut history = load_self_debug_history(&ctx.managed_root_dir, &name);
    sort_history_entries(&mut history);
    HttpResponse::Ok().json(VmSelfDebugHistoryResponse { name, history })
}

pub(crate) async fn get_vm_self_debug_history_detail(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmSelfDebugRunDetailQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let run_id = query.run_id.trim();
    if run_id.is_empty() {
        return HttpResponse::BadRequest().body("run_id is empty");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let history = load_self_debug_history(&ctx.managed_root_dir, &name);
    let Some(entry) = history.into_iter().find(|x| x.summary.run_id == run_id) else {
        return HttpResponse::NotFound().body("history run not found");
    };
    HttpResponse::Ok().json(VmSelfDebugHistoryDetailResponse {
        name,
        run_id: run_id.to_string(),
        entry,
    })
}

pub(crate) async fn archive_vm_self_debug_history(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugHistoryArchivePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut history = load_self_debug_history(&ctx.managed_root_dir, &name);
    let now = now_unix();
    let (archived_runs, removed_tasks) = archive_completed_self_debug_runs(&mut items, &mut history, now);
    sort_history_entries(&mut history);
    trim_history_entries(&mut history, VM_HISTORY_MAX_ENTRIES);
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    if let Err(e) = save_self_debug_history(&ctx.managed_root_dir, &name, &history) {
        return HttpResponse::InternalServerError().body(format!("save history failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!(
            "self-debug history archive_completed archived_runs={} removed_tasks={}",
            archived_runs, removed_tasks
        ),
    );
    HttpResponse::Ok().json(VmSelfDebugHistoryArchiveResponse {
        name,
        archived_runs,
        removed_tasks,
        history_total: history.len(),
    })
}

pub(crate) async fn clear_vm_self_debug_history(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugHistoryClearPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut history = load_self_debug_history(&ctx.managed_root_dir, &name);
    let before = history.len();
    let run_id = body.run_id.trim();
    if run_id.is_empty() {
        history.clear();
    } else {
        history.retain(|x| x.summary.run_id != run_id);
    }
    let removed = before.saturating_sub(history.len());
    if let Err(e) = save_self_debug_history(&ctx.managed_root_dir, &name, &history) {
        return HttpResponse::InternalServerError().body(format!("save history failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!(
            "self-debug history clear run_id={} removed={}",
            if run_id.is_empty() { "*" } else { run_id },
            removed
        ),
    );
    HttpResponse::Ok().json(VmSelfDebugHistoryClearResponse {
        name,
        removed,
        history_total: history.len(),
    })
}

pub(crate) async fn get_vm_self_debug_run_detail(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmSelfDebugRunDetailQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let run_id = query.run_id.trim();
    if run_id.is_empty() {
        return HttpResponse::BadRequest().body("run_id is empty");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let items = load_exec_queue(&ctx.managed_root_dir, &name);
    let tasks: Vec<VmSelfDebugRunTaskDetail> = items
        .into_iter()
        .filter(|x| {
            (x.run_kind == "self_debug"
                || x.run_kind == "self_debug_strategy"
                || x.run_kind == "self_debug_verify_after_strategy")
                && x.run_id == run_id
        })
        .map(|x| VmSelfDebugRunTaskDetail {
            id: x.id,
            run_kind: x.run_kind,
            status: x.status,
            exit_code: x.exit_code,
            command: x.command,
            message: x.message,
            output_preview: x.output_preview,
            failure_category: x.failure_category,
            failure_signature: x.failure_signature,
            failure_key_lines: x.failure_key_lines,
            command_risk_level: x.command_risk_level,
            command_risk_tags: x.command_risk_tags,
            strategy_signature: x.strategy_signature,
            trigger_task_id: x.trigger_task_id,
            created_at_unix: x.created_at_unix,
            started_at_unix: x.started_at_unix,
            finished_at_unix: x.finished_at_unix,
        })
        .collect();
    if tasks.is_empty() {
        return HttpResponse::NotFound().body("self-debug run not found");
    }
    HttpResponse::Ok().json(VmSelfDebugRunDetailResponse {
        name,
        run_id: run_id.to_string(),
        tasks,
    })
}

pub(crate) async fn get_vm_self_debug_context(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmSelfDebugRunDetailQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let run_id = query.run_id.trim();
    if run_id.is_empty() {
        return HttpResponse::BadRequest().body("run_id is empty");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let items = load_exec_queue(&ctx.managed_root_dir, &name);
    if let Some(ctx_data) = build_self_debug_context_data(&items, run_id) {
        return HttpResponse::Ok().json(VmSelfDebugContextResponse {
            name,
            run_id: run_id.to_string(),
            failed_steps: ctx_data.failed_steps,
            categories: ctx_data.categories,
            key_lines: ctx_data.key_lines,
            context_text: ctx_data.context_text,
        });
    }
    let history = load_self_debug_history(&ctx.managed_root_dir, &name);
    if let Some(entry) = history.into_iter().find(|x| x.summary.run_id == run_id) {
        return HttpResponse::Ok().json(VmSelfDebugContextResponse {
            name,
            run_id: run_id.to_string(),
            failed_steps: entry.failed_steps,
            categories: entry.categories,
            key_lines: entry.key_lines,
            context_text: entry.context_text,
        });
    }
    HttpResponse::NotFound().body("no failed steps for run_id")
}

pub(crate) async fn get_vm_self_debug_strategy_stats(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmSelfDebugRunsQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let items = load_exec_queue(&ctx.managed_root_dir, &name);
    let stats = collect_self_debug_strategy_stats(&items);
    HttpResponse::Ok().json(VmSelfDebugStrategyStatsResponse { name, stats })
}

pub(crate) async fn get_vm_self_debug_strategy_rules(
    req: HttpRequest,
    data: web::Data<AppState>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let store = load_strategy_rules(&ctx.managed_root_dir);
    HttpResponse::Ok().json(VmSelfDebugStrategyRulesResponse { rules: store.rules })
}

pub(crate) async fn save_vm_self_debug_strategy_rules(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugStrategyRulesSavePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let rules = sanitize_strategy_rules(&body.rules);
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let store = VmSelfDebugStrategyRulesStore { rules };
    if let Err(e) = save_strategy_rules(&ctx.managed_root_dir, &store) {
        return HttpResponse::InternalServerError()
            .body(format!("save strategy rules failed: {e}"));
    }
    HttpResponse::Ok().json(VmSelfDebugStrategyRulesResponse { rules: store.rules })
}

pub(crate) async fn stop_vm_self_debug_run(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugStopPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let run_id = body.run_id.trim();
    if run_id.is_empty() {
        return HttpResponse::BadRequest().body("run_id is empty");
    }

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut matched = 0usize;
    let mut running = 0usize;
    for item in items.iter_mut() {
        if item.run_kind != "self_debug" || item.run_id != run_id {
            continue;
        }
        matched += 1;
        if item.status == "pending" {
            item.status = "canceled".to_string();
            item.finished_at_unix = now_unix();
            item.exit_code = -2;
            item.message = "canceled by self-debug run stop".to_string();
        } else if item.status == "running" {
            running += 1;
        }
    }
    if matched == 0 {
        return HttpResponse::NotFound().body("self-debug run not found");
    }
    if running > 0 {
        let cancel_path = vm_exec_cancel_path(&ctx.managed_root_dir, &name);
        let _ = fs::write(&cancel_path, b"1");
    }
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("self-debug run stop requested run_id={} matched={} running={}", run_id, matched, running),
    );
    let runs = collect_self_debug_runs(&items);
    HttpResponse::Ok().json(VmSelfDebugRunsResponse { name, runs })
}

pub(crate) async fn pause_vm_self_debug_run(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugStopPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let run_id = body.run_id.trim();
    if run_id.is_empty() {
        return HttpResponse::BadRequest().body("run_id is empty");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut matched = 0usize;
    for item in items.iter_mut() {
        if item.run_id != run_id {
            continue;
        }
        if item.run_kind != "self_debug"
            && item.run_kind != "self_debug_strategy"
            && item.run_kind != "self_debug_verify_after_strategy"
        {
            continue;
        }
        if item.status == "pending" {
            item.status = "paused".to_string();
            item.message = "paused by run controller".to_string();
            matched += 1;
        }
    }
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("self-debug run pause requested run_id={} paused={}", run_id, matched),
    );
    let runs = collect_self_debug_runs(&items);
    HttpResponse::Ok().json(VmSelfDebugRunsResponse { name, runs })
}

pub(crate) async fn resume_vm_self_debug_run(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmSelfDebugStopPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let run_id = body.run_id.trim();
    if run_id.is_empty() {
        return HttpResponse::BadRequest().body("run_id is empty");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut matched = 0usize;
    for item in items.iter_mut() {
        if item.run_id != run_id {
            continue;
        }
        if item.run_kind != "self_debug"
            && item.run_kind != "self_debug_strategy"
            && item.run_kind != "self_debug_verify_after_strategy"
        {
            continue;
        }
        if item.status == "paused" {
            item.status = "pending".to_string();
            item.message = "resumed by run controller".to_string();
            matched += 1;
        }
    }
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    drop(_guard);
    if matched > 0 {
        let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
        let _ = dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("self-debug run resume requested run_id={} resumed={}", run_id, matched),
    );
    let runs = collect_self_debug_runs(&items);
    HttpResponse::Ok().json(VmSelfDebugRunsResponse { name, runs })
}

pub(crate) async fn cancel_vm_exec_task(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmQueueCancelPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };
    let task_id = body.task_id.trim();
    if task_id.is_empty() {
        return HttpResponse::BadRequest().body("task id is empty");
    }

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut items = load_exec_queue(&ctx.managed_root_dir, &name);
    let mut found = false;
    for item in items.iter_mut() {
        if item.id != task_id {
            continue;
        }
        found = true;
        if item.status == "pending" {
            item.status = "canceled".to_string();
            item.finished_at_unix = now_unix();
            item.exit_code = -2;
            item.message = "canceled before run".to_string();
        } else if item.status == "running" {
            let cancel_path = vm_exec_cancel_path(&ctx.managed_root_dir, &name);
            let _ = fs::write(&cancel_path, b"1");
        }
        break;
    }
    if !found {
        return HttpResponse::NotFound().body("task not found");
    }
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("queue cancel requested task_id={task_id}"),
    );
    HttpResponse::Ok().json(VmQueueResponse { name, items })
}

pub(crate) async fn run_next_vm_exec(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmQueueQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };

    let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    match run_next_vm_exec_core(
        &ctx.managed_root_dir,
        &name,
        &data.projects_lock,
        running_limit,
    ) {
        Ok(Some(resp)) => {
            let _ =
                dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
            HttpResponse::Ok().json(resp)
        }
        Ok(None) => HttpResponse::BadRequest().body("no runnable queued task"),
        Err(e) => HttpResponse::BadRequest().body(e),
    }
}

pub(crate) async fn dispatch_vm_exec(
    req: HttpRequest,
    data: web::Data<AppState>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    let started_workers =
        dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
    let running_total_all_vms = {
        let _guard = lock_recover(&data.projects_lock, "projects_lock");
        let state = load_vm_state(&ctx.managed_root_dir);
        count_running_exec_tasks_all_vms(&ctx.managed_root_dir, &state)
    };
    HttpResponse::Ok().json(VmExecDispatchResponse {
        started_workers,
        running_total_all_vms,
        running_limit_all_vms: running_limit,
    })
}

pub(crate) async fn get_vm_exec_dispatch_trace(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmExecDispatchTraceQuery>,
) -> impl Responder {
    let _ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let mut entries = load_dispatch_trace(&_ctx.managed_root_dir);
    entries.reverse();
    entries.truncate(limit);
    HttpResponse::Ok().json(VmExecDispatchTraceResponse { entries })
}

pub(crate) async fn scan_vm_health(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmHealthScanPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let self_heal = body.self_heal;
    if self_heal && !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let scanned = state.vms.len();
    let (issues, actions) = scan_vm_health_locked(&ctx.managed_root_dir, &mut state, self_heal);
    let _ = save_vm_state(&ctx.managed_root_dir, &state);
    HttpResponse::Ok().json(VmHealthScanResponse {
        scanned,
        issues,
        actions,
    })
}

pub(crate) async fn get_vm_policy(
    req: HttpRequest,
    data: web::Data<AppState>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    HttpResponse::Ok().json(load_vm_policy_config(&ctx.managed_root_dir))
}

pub(crate) async fn save_vm_policy(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmPolicyConfig>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let cfg = sanitize_vm_policy_config(body.into_inner());
    match save_vm_policy_config(&ctx.managed_root_dir, &cfg) {
        Ok(_) => HttpResponse::Ok().json(cfg),
        Err(e) => HttpResponse::InternalServerError().body(format!("save vm policy failed: {e}")),
    }
}

pub(crate) async fn check_vm_ready(
    req: HttpRequest,
    data: web::Data<AppState>,
    query: web::Query<VmReadyQuery>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let name = match sanitize_vm_name(&query.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };
    reconcile_vm_power_state(&ctx.managed_root_dir, vm);
    if vm.backend != "qemu" {
        return HttpResponse::Ok().json(VmReadyResponse {
            ok: false,
            message: "ready check is only available for qemu backend".to_string(),
        });
    }
    if vm.power_state != "running" {
        return HttpResponse::Ok().json(VmReadyResponse {
            ok: false,
            message: "vm is not running".to_string(),
        });
    }
    let ssh_port = vm.ssh_port.unwrap_or(0);
    if ssh_port == 0 {
        return HttpResponse::Ok().json(VmReadyResponse {
            ok: false,
            message: "vm ssh port is not set".to_string(),
        });
    }
    let ssh_user = normalize_ssh_user(&vm.ssh_user);
    let check = run_ssh_command(&ssh_user, ssh_port, "echo VM_READY", 8);
    match check {
        Ok(r) if r.ok => {
            vm.last_message = "vm ssh is ready".to_string();
            vm.updated_at_unix = now_unix();
            append_vm_log(&ctx.managed_root_dir, &vm.name, "ready check passed");
            let _ = save_vm_state(&ctx.managed_root_dir, &state);
            HttpResponse::Ok().json(VmReadyResponse {
                ok: true,
                message: "vm ssh is ready".to_string(),
            })
        }
        Ok(r) => {
            let msg = format!("ready check failed (exit={})", r.exit_code);
            append_vm_log(&ctx.managed_root_dir, &vm.name, &msg);
            HttpResponse::Ok().json(VmReadyResponse {
                ok: false,
                message: msg,
            })
        }
        Err(e) => {
            append_vm_log(
                &ctx.managed_root_dir,
                &vm.name,
                &format!("ready check error: {e}"),
            );
            HttpResponse::Ok().json(VmReadyResponse {
                ok: false,
                message: format!("ready check error: {e}"),
            })
        }
    }
}

pub(crate) async fn provision_vm(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmProvisionPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    if let Err(e) = web_ui_authz::ensure_user_dirs(&ctx) {
        return HttpResponse::InternalServerError().body(format!("init user dirs failed: {e}"));
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    if state.vms.contains_key(&name) {
        return HttpResponse::Conflict().body("vm already exists");
    }
    let vm_limit = vm_instance_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    if state.vms.len() >= vm_limit {
        return HttpResponse::TooManyRequests()
            .body(format!("vm instance limit exceeded: limit={vm_limit}"));
    }

    let cpu = default_cpu(body.cpu);
    let memory_mb = default_memory_mb(body.memory_mb);
    let disk_gb = default_disk_gb(body.disk_gb);
    let backend = normalize_backend(&body.backend).to_string();

    let disk_dir = vm_disk_dir(&ctx.managed_root_dir);
    if let Err(e) = fs::create_dir_all(&disk_dir) {
        return HttpResponse::InternalServerError().body(format!("create vm disk dir failed: {e}"));
    }
    let disk_path = disk_dir.join(format!("{name}.qcow2"));
    let disk_create_message = match create_disk_image(&disk_path, disk_gb) {
        Ok(v) => v,
        Err(e) => return HttpResponse::InternalServerError().body(format!("create disk failed: {e}")),
    };

    let now = now_unix();
    let instance = VmInstance {
        name: name.clone(),
        backend,
        power_state: "stopped".to_string(),
        cpu,
        memory_mb,
        disk_gb,
        disk_path: disk_path
            .strip_prefix(workspace_root())
            .unwrap_or(&disk_path)
            .display()
            .to_string(),
        os_image: body.os_image.trim().to_string(),
        created_at_unix: now,
        updated_at_unix: now,
        last_message: disk_create_message,
        process_id: None,
        ssh_port: None,
        ssh_user: normalize_ssh_user(&body.ssh_user),
    };
    state.vms.insert(name, instance.clone());
    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &instance.name,
        &format!(
            "vm provisioned backend={} cpu={} mem_mb={} disk_gb={}",
            instance.backend, instance.cpu, instance.memory_mb, instance.disk_gb
        ),
    );
    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective: true,
        message: "vm provisioned".to_string(),
        vm: Some(instance),
    })
}

pub(crate) async fn start_vm(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmActionPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    for vm in state.vms.values_mut() {
        let _ = reconcile_vm_power_state(&ctx.managed_root_dir, vm);
    }
    let Some(current_vm) = state.vms.get(&name).cloned() else {
        return HttpResponse::NotFound().body("vm not found");
    };
    if current_vm.power_state == "running" {
        append_vm_log(
            &ctx.managed_root_dir,
            &current_vm.name,
            "start requested but vm already running",
        );
        return HttpResponse::Ok().json(VmActionResponse {
            ok: true,
            effective: true,
            message: "vm already running".to_string(),
            vm: Some(current_vm),
        });
    }
    let running_limit = vm_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
    let running_now = state
        .vms
        .values()
        .filter(|x| x.power_state == "running")
        .count();
    if running_now >= running_limit {
        return HttpResponse::TooManyRequests().body(format!(
            "running vm limit exceeded: running={} limit={}",
            running_now, running_limit
        ));
    }
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    let (effective, msg) = match vm.backend.as_str() {
        "qemu" => {
            let ssh_port = vm.ssh_port.or_else(allocate_local_port).unwrap_or(2222);
            match start_qemu_process(&ctx.managed_root_dir, vm, ssh_port) {
            Ok((pid, m)) => {
                vm.process_id = Some(pid);
                vm.power_state = "running".to_string();
                vm.ssh_port = Some(ssh_port);
                (true, m)
            }
            Err(e) => {
                vm.process_id = None;
                vm.power_state = "stopped".to_string();
                vm.last_message = e.clone();
                append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("start failed: {e}"));
                if let Err(se) = save_vm_state(&ctx.managed_root_dir, &state) {
                    return HttpResponse::InternalServerError().body(format!("save vm state failed: {se}"));
                }
                return HttpResponse::InternalServerError().body(e);
            }
            }
        }
        "libvirt" => (
            false,
            "libvirt execution not wired yet; lifecycle remains metadata-only".to_string(),
        ),
        _ => (
            false,
            "metadata-only start completed; hypervisor execution not enabled for this backend".to_string(),
        ),
    };

    vm.updated_at_unix = now_unix();
    vm.last_message = msg.clone();
    append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("start result: {msg}"));
    let snapshot = vm.clone();
    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }
    drop(_guard);
    if effective {
        let running_limit = vm_exec_running_limit_by_role(&ctx.managed_root_dir, &ctx.role);
        let _ = dispatch_vm_exec_workers(&ctx.managed_root_dir, data.projects_lock.clone(), running_limit);
    }
    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective,
        message: msg,
        vm: Some(snapshot),
    })
}

pub(crate) async fn stop_vm(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmActionPayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    let (effective, msg) = match vm.backend.as_str() {
        "qemu" => match stop_qemu_process(&ctx.managed_root_dir, vm) {
            Ok(m) => {
                vm.process_id = None;
                vm.power_state = "stopped".to_string();
                (true, m)
            }
            Err(e) => {
                vm.last_message = e.clone();
                append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("stop failed: {e}"));
                if let Err(se) = save_vm_state(&ctx.managed_root_dir, &state) {
                    return HttpResponse::InternalServerError().body(format!("save vm state failed: {se}"));
                }
                return HttpResponse::InternalServerError().body(e);
            }
        },
        "libvirt" => (
            false,
            "libvirt stop is not wired yet; lifecycle remains metadata-only".to_string(),
        ),
        _ => (
            false,
            "metadata-only stop completed; hypervisor execution not enabled for this backend".to_string(),
        ),
    };

    vm.updated_at_unix = now_unix();
    vm.last_message = msg.clone();
    append_vm_log(&ctx.managed_root_dir, &vm.name, &format!("stop result: {msg}"));
    let snapshot = vm.clone();
    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }
    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective,
        message: msg,
        vm: Some(snapshot),
    })
}

pub(crate) async fn delete_vm(
    req: HttpRequest,
    data: web::Data<AppState>,
    body: web::Json<VmDeletePayload>,
) -> impl Responder {
    let ctx = match web_ui_authz::user_ctx_for_request(&req, &data) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if !ctx.can_write {
        return HttpResponse::Forbidden().body("read-only session");
    }
    let name = match sanitize_vm_name(&body.name) {
        Some(v) => v,
        None => return HttpResponse::BadRequest().body("invalid vm name"),
    };

    let _guard = lock_recover(&data.projects_lock, "projects_lock");
    let mut state = load_vm_state(&ctx.managed_root_dir);
    let Some(vm) = state.vms.remove(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    if vm.backend == "qemu" {
        let _ = stop_qemu_process(&ctx.managed_root_dir, &vm);
    }

    if let Err(e) = save_vm_state(&ctx.managed_root_dir, &state) {
        return HttpResponse::InternalServerError().body(format!("save vm state failed: {e}"));
    }

    if body.purge_disk {
        let disk_path = abs_path_from_rel_or_abs(&vm.disk_path);
        let _ = fs::remove_file(disk_path);
    }
    let _ = fs::remove_file(vm_pid_path(&ctx.managed_root_dir, &vm.name));
    append_vm_log(
        &ctx.managed_root_dir,
        &vm.name,
        &format!("vm deleted purge_disk={}", body.purge_disk),
    );

    HttpResponse::Ok().json(VmActionResponse {
        ok: true,
        effective: true,
        message: "vm deleted".to_string(),
        vm: Some(vm),
    })
}

#[cfg(test)]
mod tests {
    use actix_web::{http::header, web, App};
    use actix_web::test as awtest;
    use crossbeam_channel::unbounded;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        archive_completed_self_debug_runs, collect_self_debug_runs, default_cpu, default_disk_gb,
        default_memory_mb, enforce_self_debug_run_timeout, ensure_queue_capacity, is_queue_active_status,
        is_running_task_hard_timed_out, load_exec_queue, normalize_backend, now_unix,
        priority_aging_boost, queue_has_active_duplicate, queue_item_from_values,
        recover_stale_running_tasks_in_queue, sanitize_self_debug_run_id, sanitize_vm_name,
        save_exec_queue, save_vm_state, load_vm_state, select_dispatch_vm_candidates_with_score, load_dispatch_trace,
        append_dispatch_trace_entry, scan_vm_health_locked, VM_DISPATCH_TRACE_MAX_ENTRIES,
        load_vm_policy_config, save_vm_policy_config,
        strategy_priority_boost_by_stats, queue_watchdog_recovery_stats,
        trim_history_entries, vm_instance_limit_by_role, vm_queue_limit_by_role,
        vm_running_limit_by_role, vm_exec_running_limit_by_role,
        VmSelfDebugHistoryEntry,
    };

    #[test]
    fn sanitize_vm_name_accepts_safe_chars() {
        assert_eq!(
            sanitize_vm_name("vm-01_alpha.test"),
            Some("vm-01_alpha.test".to_string())
        );
    }

    #[test]
    fn sanitize_vm_name_rejects_unsafe_input() {
        assert!(sanitize_vm_name("../escape").is_none());
        assert!(sanitize_vm_name("with space").is_none());
        assert!(sanitize_vm_name("").is_none());
    }

    #[test]
    fn vm_resource_defaults_are_bounded() {
        assert_eq!(default_cpu(0), 2);
        assert_eq!(default_memory_mb(0), 4096);
        assert_eq!(default_disk_gb(0), 40);
        assert_eq!(default_cpu(100), 32);
        assert_eq!(default_memory_mb(999_999), 262_144);
        assert_eq!(default_disk_gb(9), 10);
    }

    #[test]
    fn normalize_backend_maps_unknown_to_metadata() {
        assert_eq!(normalize_backend("qemu"), "qemu");
        assert_eq!(normalize_backend("libvirt"), "libvirt");
        assert_eq!(normalize_backend("foobar"), "metadata-only");
        assert_eq!(normalize_backend(""), "metadata-only");
    }

    #[test]
    fn sanitize_self_debug_run_id_checks_format() {
        assert_eq!(
            sanitize_self_debug_run_id("run_01-A"),
            Some("run_01-A".to_string())
        );
        assert!(sanitize_self_debug_run_id("bad id").is_none());
        assert!(sanitize_self_debug_run_id("").is_none());
    }

    #[test]
    fn self_debug_run_summary_counts_paused() {
        let mut a = queue_item_from_values("echo a", 10, 0, 0, 0);
        a.run_id = "sd-1".to_string();
        a.run_kind = "self_debug".to_string();
        a.status = "paused".to_string();

        let mut b = queue_item_from_values("echo b", 10, 0, 0, 0);
        b.run_id = "sd-1".to_string();
        b.run_kind = "self_debug".to_string();
        b.status = "pending".to_string();

        let runs = collect_self_debug_runs(&[a, b]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "sd-1");
        assert_eq!(runs[0].paused, 1);
        assert_eq!(runs[0].pending, 1);
    }

    #[test]
    fn strategy_priority_boost_uses_success_rate_with_min_attempts() {
        let mut items = Vec::new();
        for i in 0..5 {
            let mut trigger = queue_item_from_values("bash scripts/run_tests.sh", 30, 0, 0, 0);
            trigger.id = format!("t-{i}");
            trigger.run_kind = "self_debug".to_string();
            trigger.failure_category = "test_failure".to_string();
            trigger.status = "failed".to_string();
            items.push(trigger);

            let mut verify =
                queue_item_from_values("bash scripts/run_tests.sh", 30, 0, 0, 0);
            verify.run_kind = "self_debug_verify_after_strategy".to_string();
            verify.trigger_task_id = format!("t-{i}");
            verify.status = if i == 4 {
                "failed".to_string()
            } else {
                "done".to_string()
            };
            items.push(verify);
        }
        assert_eq!(strategy_priority_boost_by_stats(&items, "test_failure"), 3);

        let mut small = Vec::new();
        for i in 0..2 {
            let mut trigger = queue_item_from_values("bash scripts/run_tests.sh", 30, 0, 0, 0);
            trigger.id = format!("s-{i}");
            trigger.run_kind = "self_debug".to_string();
            trigger.failure_category = "build_error".to_string();
            trigger.status = "failed".to_string();
            small.push(trigger);

            let mut verify =
                queue_item_from_values("bash scripts/run_tests.sh", 30, 0, 0, 0);
            verify.run_kind = "self_debug_verify_after_strategy".to_string();
            verify.trigger_task_id = format!("s-{i}");
            verify.status = "done".to_string();
            small.push(verify);
        }
        assert_eq!(strategy_priority_boost_by_stats(&small, "build_error"), 0);
    }

    #[test]
    fn enforce_self_debug_run_timeout_cancels_remaining_tasks() {
        let mut done = queue_item_from_values("echo done", 10, 0, 0, 0);
        done.run_id = "sd-timeout".to_string();
        done.run_kind = "self_debug".to_string();
        done.run_max_runtime_sec = 30;
        done.created_at_unix = 100;
        done.status = "done".to_string();

        let mut pending = queue_item_from_values("echo pending", 10, 0, 0, 0);
        pending.run_id = "sd-timeout".to_string();
        pending.run_kind = "self_debug_strategy".to_string();
        pending.run_max_runtime_sec = 30;
        pending.created_at_unix = 100;
        pending.status = "pending".to_string();

        let mut items = vec![done, pending];
        let hit = enforce_self_debug_run_timeout(&mut items, "sd-timeout", 131);
        assert_eq!(hit, Some(30));
        assert_eq!(items[1].status, "canceled");
    }

    #[test]
    fn archive_completed_self_debug_runs_moves_done_run_to_history() {
        let mut done = queue_item_from_values("echo done", 10, 0, 0, 0);
        done.run_id = "sd-done".to_string();
        done.run_kind = "self_debug".to_string();
        done.status = "done".to_string();
        done.created_at_unix = 100;
        done.finished_at_unix = 110;

        let mut pending = queue_item_from_values("echo pending", 10, 0, 0, 0);
        pending.run_id = "sd-pending".to_string();
        pending.run_kind = "self_debug".to_string();
        pending.status = "pending".to_string();
        pending.created_at_unix = 100;

        let mut items = vec![done, pending];
        let mut history = Vec::new();
        let (runs, removed_tasks) = archive_completed_self_debug_runs(&mut items, &mut history, 200);
        assert_eq!(runs, 1);
        assert_eq!(removed_tasks, 1);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].summary.run_id, "sd-done");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].run_id, "sd-pending");
    }

    #[test]
    fn trim_history_entries_keeps_latest_count() {
        let mut entries = vec![
            VmSelfDebugHistoryEntry {
                summary: crate::web_ui_models::VmSelfDebugRunSummary {
                    run_id: "a".to_string(),
                    total: 1,
                    pending: 0,
                    paused: 0,
                    running: 0,
                    done: 1,
                    failed: 0,
                    canceled: 0,
                    updated_at_unix: 1,
                },
                archived_at_unix: 3,
                failed_steps: 0,
                categories: Vec::new(),
                key_lines: Vec::new(),
                context_text: String::new(),
            },
            VmSelfDebugHistoryEntry {
                summary: crate::web_ui_models::VmSelfDebugRunSummary {
                    run_id: "b".to_string(),
                    total: 1,
                    pending: 0,
                    paused: 0,
                    running: 0,
                    done: 1,
                    failed: 0,
                    canceled: 0,
                    updated_at_unix: 2,
                },
                archived_at_unix: 2,
                failed_steps: 0,
                categories: Vec::new(),
                key_lines: Vec::new(),
                context_text: String::new(),
            },
            VmSelfDebugHistoryEntry {
                summary: crate::web_ui_models::VmSelfDebugRunSummary {
                    run_id: "c".to_string(),
                    total: 1,
                    pending: 0,
                    paused: 0,
                    running: 0,
                    done: 1,
                    failed: 0,
                    canceled: 0,
                    updated_at_unix: 3,
                },
                archived_at_unix: 1,
                failed_steps: 0,
                categories: Vec::new(),
                key_lines: Vec::new(),
                context_text: String::new(),
            },
        ];
        trim_history_entries(&mut entries, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].summary.run_id, "a");
        assert_eq!(entries[1].summary.run_id, "b");
    }

    #[test]
    fn archive_history_captures_context_snapshot() {
        let mut failed = queue_item_from_values("bash scripts/run_tests.sh", 30, 0, 0, 0);
        failed.run_id = "sd-fail".to_string();
        failed.run_kind = "self_debug".to_string();
        failed.status = "failed".to_string();
        failed.failure_category = "test_failure".to_string();
        failed.failure_key_lines = vec!["assertion failed: x != y".to_string()];
        failed.output_preview = "stderr:\nassertion failed".to_string();
        failed.message = "test failed".to_string();

        let mut items = vec![failed];
        let mut history = Vec::new();
        let (runs, removed_tasks) = archive_completed_self_debug_runs(&mut items, &mut history, 300);
        assert_eq!(runs, 1);
        assert_eq!(removed_tasks, 1);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].summary.run_id, "sd-fail");
        assert_eq!(history[0].failed_steps, 1);
        assert!(history[0].context_text.contains("run_id=sd-fail"));
        assert!(history[0].context_text.contains("key_failure_lines"));
    }

    #[test]
    fn role_limits_and_capacity_checks_work() {
        let root = "autocoding_data/test_role_limits";
        assert!(
            vm_instance_limit_by_role(root, &crate::auth::AccountRole::Admin)
                > vm_instance_limit_by_role(root, &crate::auth::AccountRole::User)
        );
        assert!(
            vm_queue_limit_by_role(root, &crate::auth::AccountRole::Admin)
                > vm_queue_limit_by_role(root, &crate::auth::AccountRole::User)
        );
        assert!(
            vm_running_limit_by_role(root, &crate::auth::AccountRole::Admin)
                > vm_running_limit_by_role(root, &crate::auth::AccountRole::User)
        );
        assert!(
            vm_exec_running_limit_by_role(root, &crate::auth::AccountRole::Admin)
                > vm_exec_running_limit_by_role(root, &crate::auth::AccountRole::User)
        );
        assert!(ensure_queue_capacity(10, 5, 20).is_ok());
        assert!(ensure_queue_capacity(20, 1, 20).is_err());
    }

    #[test]
    fn queue_active_status_and_duplicate_detection_work() {
        assert!(is_queue_active_status("pending"));
        assert!(is_queue_active_status("running"));
        assert!(is_queue_active_status("paused"));
        assert!(!is_queue_active_status("done"));

        let mut done = queue_item_from_values("echo hi", 10, 0, 0, 0);
        done.status = "done".to_string();
        let mut pending = queue_item_from_values("echo hi", 10, 0, 0, 0);
        pending.status = "pending".to_string();
        let items = vec![done, pending];
        assert!(queue_has_active_duplicate(&items, "echo hi"));
        assert!(!queue_has_active_duplicate(&items, "echo bye"));
    }

    #[test]
    fn stale_running_task_is_force_recovered() {
        let mut running = queue_item_from_values("sleep 120", 10, 0, 0, 0);
        running.status = "running".to_string();
        running.started_at_unix = 100;

        let mut fresh = queue_item_from_values("echo ok", 30, 0, 0, 0);
        fresh.status = "running".to_string();
        fresh.started_at_unix = 130;

        let mut items = vec![running, fresh];
        assert!(is_running_task_hard_timed_out(&items[0], 126, 15));
        assert!(!is_running_task_hard_timed_out(&items[1], 126, 15));
        let recovered = recover_stale_running_tasks_in_queue(&mut items, 126, 15);
        assert_eq!(recovered, 1);
        assert_eq!(items[0].status, "failed");
        assert_eq!(items[0].exit_code, -1);
        assert_eq!(items[1].status, "running");
    }

    #[test]
    fn queue_watchdog_stats_count_and_last_timestamp() {
        let mut a = queue_item_from_values("sleep 1", 5, 0, 0, 0);
        a.message = "watchdog hard-timeout recovered task (timeout=5s+15s)".to_string();
        a.finished_at_unix = 120;
        let mut b = queue_item_from_values("echo ok", 5, 0, 0, 0);
        b.message = "done".to_string();
        b.finished_at_unix = 121;
        let mut c = queue_item_from_values("sleep 2", 5, 0, 0, 0);
        c.message = "watchdog hard-timeout recovered task (timeout=5s+15s)".to_string();
        c.finished_at_unix = 140;
        let (total, last) = queue_watchdog_recovery_stats(&[a, b, c]);
        assert_eq!(total, 2);
        assert_eq!(last, 140);
    }

    #[test]
    fn dispatch_candidates_follow_priority_then_wait_time() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let managed_root = format!("autocoding_data/users/dispatch_test_{unique}");
        let now = now_unix();
        let mut state_store = crate::web_ui_models::VmStateStore::default();
        for vm_name in ["vm-a", "vm-b", "vm-c"] {
            state_store.vms.insert(
                vm_name.to_string(),
                crate::web_ui_models::VmInstance {
                    name: vm_name.to_string(),
                    backend: "qemu".to_string(),
                    power_state: "running".to_string(),
                    cpu: 2,
                    memory_mb: 1024,
                    disk_gb: 10,
                    disk_path: format!("autocoding_data/{vm_name}.qcow2"),
                    os_image: "linux".to_string(),
                    created_at_unix: now,
                    updated_at_unix: now,
                    last_message: String::new(),
                    process_id: Some(1),
                    ssh_port: Some(2222),
                    ssh_user: "root".to_string(),
                },
            );
        }
        save_vm_state(&managed_root, &state_store).expect("save vm state");

        let mut a = queue_item_from_values("echo a", 30, 0, 10, 0);
        a.status = "pending".to_string();
        a.created_at_unix = now.saturating_sub(5);
        save_exec_queue(&managed_root, "vm-a", &[a]).expect("save queue a");

        let mut b = queue_item_from_values("echo b", 30, 0, 10, 0);
        b.status = "pending".to_string();
        b.created_at_unix = now.saturating_sub(20);
        save_exec_queue(&managed_root, "vm-b", &[b]).expect("save queue b");

        let mut c = queue_item_from_values("echo c", 30, 0, 50, 0);
        c.status = "pending".to_string();
        c.created_at_unix = now.saturating_sub(1);
        save_exec_queue(&managed_root, "vm-c", &[c]).expect("save queue c");

        let top2: Vec<String> = select_dispatch_vm_candidates_with_score(&managed_root, &state_store, now)
            .into_iter()
            .take(2)
            .map(|x| x.vm_name)
            .collect();
        assert_eq!(top2, vec!["vm-c".to_string(), "vm-b".to_string()]);
    }

    #[test]
    fn priority_aging_boost_is_bounded() {
        assert_eq!(priority_aging_boost(100, 100, 60, 20), 0);
        assert_eq!(priority_aging_boost(100, 159, 60, 20), 0);
        assert_eq!(priority_aging_boost(100, 160, 60, 20), 1);
        assert_eq!(priority_aging_boost(100, 100 + 3600, 60, 20), 20);
    }

    #[test]
    fn dispatch_candidates_apply_aging_boost() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let managed_root = format!("autocoding_data/users/dispatch_aging_{unique}");
        let now = now_unix();
        let mut state_store = crate::web_ui_models::VmStateStore::default();
        for vm_name in ["vm-new-high", "vm-old-low"] {
            state_store.vms.insert(
                vm_name.to_string(),
                crate::web_ui_models::VmInstance {
                    name: vm_name.to_string(),
                    backend: "qemu".to_string(),
                    power_state: "running".to_string(),
                    cpu: 2,
                    memory_mb: 1024,
                    disk_gb: 10,
                    disk_path: format!("autocoding_data/{vm_name}.qcow2"),
                    os_image: "linux".to_string(),
                    created_at_unix: now,
                    updated_at_unix: now,
                    last_message: String::new(),
                    process_id: Some(1),
                    ssh_port: Some(2222),
                    ssh_user: "root".to_string(),
                },
            );
        }
        save_vm_state(&managed_root, &state_store).expect("save vm state");

        let mut high = queue_item_from_values("echo high", 30, 0, 10, 0);
        high.status = "pending".to_string();
        high.created_at_unix = now.saturating_sub(30);
        save_exec_queue(&managed_root, "vm-new-high", &[high]).expect("save queue high");

        let mut low = queue_item_from_values("echo low", 30, 0, -5, 0);
        low.status = "pending".to_string();
        low.created_at_unix = now.saturating_sub(16 * 60);
        save_exec_queue(&managed_root, "vm-old-low", &[low]).expect("save queue low");

        let top: Vec<String> = select_dispatch_vm_candidates_with_score(&managed_root, &state_store, now)
            .into_iter()
            .take(1)
            .map(|x| x.vm_name)
            .collect();
        assert_eq!(top, vec!["vm-old-low".to_string()]);
    }

    #[test]
    fn dispatch_trace_keeps_ring_buffer_size() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let managed_root = format!("autocoding_data/users/dispatch_trace_{unique}");
        for i in 0..(VM_DISPATCH_TRACE_MAX_ENTRIES + 7) {
            append_dispatch_trace_entry(
                &managed_root,
                crate::web_ui_models::VmExecDispatchTraceEntry {
                    created_at_unix: i as u64,
                    running_total_before: 0,
                    running_limit: 4,
                    available_slots: 4,
                    selected_vms: Vec::new(),
                    candidates: Vec::new(),
                },
            );
        }
        let entries = load_dispatch_trace(&managed_root);
        assert_eq!(entries.len(), VM_DISPATCH_TRACE_MAX_ENTRIES);
        assert_eq!(entries[0].created_at_unix, 7);
    }

    #[test]
    fn health_scan_self_heal_recovers_stale_running_tasks() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let managed_root = format!("autocoding_data/users/health_scan_{unique}");
        let now = now_unix();
        let vm_name = "vm-health".to_string();
        let mut state = crate::web_ui_models::VmStateStore::default();
        state.vms.insert(
            vm_name.clone(),
            crate::web_ui_models::VmInstance {
                name: vm_name.clone(),
                backend: "metadata-only".to_string(),
                power_state: "stopped".to_string(),
                cpu: 2,
                memory_mb: 1024,
                disk_gb: 10,
                disk_path: "autocoding_data/dummy.qcow2".to_string(),
                os_image: "linux".to_string(),
                created_at_unix: now,
                updated_at_unix: now,
                last_message: String::new(),
                process_id: None,
                ssh_port: None,
                ssh_user: "root".to_string(),
            },
        );
        save_vm_state(&managed_root, &state).expect("save vm state");

        let mut running = queue_item_from_values("sleep 120", 10, 0, 0, 0);
        running.status = "running".to_string();
        running.started_at_unix = now.saturating_sub(60);
        save_exec_queue(&managed_root, &vm_name, &[running]).expect("save queue");

        let mut loaded = load_vm_state(&managed_root);
        let (issues, actions) = scan_vm_health_locked(&managed_root, &mut loaded, true);
        assert!(!issues.is_empty());
        assert!(!actions.is_empty());
        let items = load_exec_queue(&managed_root, &vm_name);
        assert_eq!(items[0].status, "failed");
    }

    #[test]
    fn vm_policy_config_persist_and_clamp() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let managed_root = format!("autocoding_data/users/policy_cfg_{unique}");
        let cfg = crate::web_ui_models::VmPolicyConfig {
            user: crate::web_ui_models::VmPolicyRoleLimits {
                vm_instance_max: 0,
                vm_queue_max_items: 0,
                vm_running_max: 0,
                vm_exec_running_max: 0,
            },
            admin: crate::web_ui_models::VmPolicyRoleLimits {
                vm_instance_max: 1,
                vm_queue_max_items: 1,
                vm_running_max: 1,
                vm_exec_running_max: 1,
            },
            scheduler: crate::web_ui_models::VmPolicyScheduler {
                exec_hard_timeout_grace_sec: 999_999,
                priority_aging_step_sec: 0,
                priority_aging_max_boost: -1,
            },
        };
        save_vm_policy_config(&managed_root, &cfg).expect("save policy");
        let loaded = load_vm_policy_config(&managed_root);
        assert_eq!(loaded.user.vm_instance_max, 1);
        assert!(loaded.admin.vm_instance_max >= loaded.user.vm_instance_max);
        assert_eq!(loaded.scheduler.priority_aging_step_sec, 1);
        assert_eq!(loaded.scheduler.priority_aging_max_boost, 0);
    }

    #[test]
    fn command_risk_classification_marks_high_and_low() {
        let (l1, t1) = super::classify_command_risk("echo hello");
        assert_eq!(l1, "low");
        assert!(t1.is_empty());

        let (l2, t2) = super::classify_command_risk("sudo apt-get install -y git");
        assert_eq!(l2, "medium");
        assert!(t2.iter().any(|x| x == "privilege"));

        let (l3, t3) = super::classify_command_risk("rm -rf /tmp/x && reboot");
        assert_eq!(l3, "high");
        assert!(t3.iter().any(|x| x == "system-power"));
    }

    #[actix_web::test]
    async fn http_self_debug_history_flow_works_end_to_end() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let username = format!("vmhist_{unique}");
        let vm_name = format!("vm_{unique}");
        let managed_root = format!("autocoding_data/users/{}", username);

        let tmp_auth_root = std::env::temp_dir().join(format!("kacf_auth_test_{unique}"));
        let auth_paths = crate::auth::AuthSystemPaths::new(&tmp_auth_root);
        let auth = crate::auth::AuthStore::new(auth_paths);
        auth.ensure_dirs().expect("ensure auth dirs");
        let user = auth
            .create_user(
                &username,
                "vmhist",
                &format!("{username}@example.com"),
                crate::auth::AccountRole::User,
                Some("pw".to_string()),
                crate::auth::types::LoginOption::PasswordOnly,
            )
            .expect("create user");
        let session = auth
            .create_session_for_user(&user)
            .expect("create session");

        let mut state_store = crate::web_ui_models::VmStateStore::default();
        let now = now_unix();
        state_store.vms.insert(
            vm_name.clone(),
            crate::web_ui_models::VmInstance {
                name: vm_name.clone(),
                backend: "metadata-only".to_string(),
                power_state: "stopped".to_string(),
                cpu: 2,
                memory_mb: 1024,
                disk_gb: 10,
                disk_path: "autocoding_data/dummy.qcow2".to_string(),
                os_image: "linux".to_string(),
                created_at_unix: now,
                updated_at_unix: now,
                last_message: String::new(),
                process_id: None,
                ssh_port: None,
                ssh_user: "root".to_string(),
            },
        );
        save_vm_state(&managed_root, &state_store).expect("save vm state");

        let (tx_req, _rx_req) = unbounded::<crate::protocol::AgentRequest>();
        let app_state = crate::web_ui::AppState {
            tx_req,
            events: Arc::new(Mutex::new(VecDeque::new())),
            event_bytes: Arc::new(Mutex::new(0)),
            next_event_id: Arc::new(Mutex::new(1)),
            runtime: Arc::new(Mutex::new(crate::web_ui_models::RuntimeStatus::default())),
            projects_lock: Arc::new(Mutex::new(())),
            debug_client_logs: crate::web_ui_debug::DebugLogStore::new(),
            stop_now: Arc::new(AtomicBool::new(false)),
            auth: auth.clone(),
        };
        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(app_state))
                .route("/vm/self_debug/start", web::post().to(super::start_vm_self_debug_plan))
                .route("/vm/self_debug/context", web::get().to(super::get_vm_self_debug_context))
                .route("/vm/self_debug/history", web::get().to(super::list_vm_self_debug_history))
                .route(
                    "/vm/self_debug/history/detail",
                    web::get().to(super::get_vm_self_debug_history_detail),
                )
                .route(
                    "/vm/self_debug/history/archive_completed",
                    web::post().to(super::archive_vm_self_debug_history),
                )
                .route(
                    "/vm/self_debug/history/clear",
                    web::post().to(super::clear_vm_self_debug_history),
                ),
        )
        .await;

        let cookie = format!("kacf_session={}", session.session_id);
        let req = awtest::TestRequest::post()
            .uri("/vm/self_debug/start")
            .insert_header((header::COOKIE, cookie.clone()))
            .set_json(json!({
                "name": vm_name,
                "fix_cmd": "echo fix",
                "cycles": 1,
                "max_task_budget": 3
            }))
            .to_request();
        let start_resp: serde_json::Value = awtest::call_and_read_body_json(&app, req).await;
        let run_id = start_resp
            .get("run_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        assert!(!run_id.is_empty());

        let mut items = load_exec_queue(&managed_root, &vm_name);
        assert!(!items.is_empty());
        items[0].status = "failed".to_string();
        items[0].finished_at_unix = now_unix();
        items[0].failure_category = "test_failure".to_string();
        items[0].failure_key_lines = vec!["assertion failed".to_string()];
        items[0].output_preview = "stderr:\nassertion failed".to_string();
        items[0].message = "failed".to_string();
        for item in items.iter_mut().skip(1) {
            item.status = "done".to_string();
            item.finished_at_unix = now_unix();
        }
        save_exec_queue(&managed_root, &vm_name, &items).expect("save queue");

        let req = awtest::TestRequest::post()
            .uri("/vm/self_debug/history/archive_completed")
            .insert_header((header::COOKIE, cookie.clone()))
            .set_json(json!({ "name": vm_name }))
            .to_request();
        let archive_resp: serde_json::Value = awtest::call_and_read_body_json(&app, req).await;
        assert!(
            archive_resp
                .get("archived_runs")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                >= 1
        );

        let req = awtest::TestRequest::get()
            .uri(&format!("/vm/self_debug/history?name={}", vm_name))
            .insert_header((header::COOKIE, cookie.clone()))
            .to_request();
        let history_resp: serde_json::Value = awtest::call_and_read_body_json(&app, req).await;
        let history = history_resp
            .get("history")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(!history.is_empty());

        let req = awtest::TestRequest::get()
            .uri(&format!(
                "/vm/self_debug/history/detail?name={}&run_id={}",
                vm_name, run_id
            ))
            .insert_header((header::COOKIE, cookie.clone()))
            .to_request();
        let detail_resp: serde_json::Value = awtest::call_and_read_body_json(&app, req).await;
        assert_eq!(
            detail_resp
                .get("entry")
                .and_then(|e| e.get("summary"))
                .and_then(|s| s.get("run_id"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            run_id
        );

        let req = awtest::TestRequest::get()
            .uri(&format!(
                "/vm/self_debug/context?name={}&run_id={}",
                vm_name, run_id
            ))
            .insert_header((header::COOKIE, cookie.clone()))
            .to_request();
        let context_resp: serde_json::Value = awtest::call_and_read_body_json(&app, req).await;
        let context_text = context_resp
            .get("context_text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(context_text.contains("run_id="));

        let req = awtest::TestRequest::post()
            .uri("/vm/self_debug/history/clear")
            .insert_header((header::COOKIE, cookie))
            .set_json(json!({ "name": vm_name, "run_id": run_id }))
            .to_request();
        let clear_resp: serde_json::Value = awtest::call_and_read_body_json(&app, req).await;
        assert_eq!(
            clear_resp
                .get("removed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            1
        );

        let _ = fs::remove_dir_all(
            std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(managed_root),
        );
        let _ = fs::remove_dir_all(tmp_auth_root);
    }
}

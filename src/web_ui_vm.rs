use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wait_timeout::ChildExt;

use crate::lock_utils::lock_recover;
use crate::web_ui::AppState;
use crate::web_ui_authz;
use crate::web_ui_models::{
    VmActionPayload, VmActionResponse, VmBootstrapPayload, VmCapability, VmClonePayload,
    VmDeletePayload, VmExecBatchTaskPayload, VmExecCancelPayload, VmExecCancelResponse,
    VmExecCustomProfileDeletePayload, VmExecCustomProfileSavePayload, VmExecEnqueueBatchPayload,
    VmExecEnqueuePayload, VmExecEnqueueProfilePayload, VmExecPayload, VmExecProfileDetailQuery,
    VmExecProfileDetailResponse, VmExecProfilePreviewQuery, VmExecProfilePreviewResponse,
    VmExecProfilesResponse, VmExecQueueItem, VmExecResponse, VmInstance, VmLogQuery,
    VmLogsResponse, VmProvisionPayload, VmQueueCancelPayload, VmQueueQuery, VmQueueResponse,
    VmQueueStatsResponse, VmReadyQuery, VmReadyResponse, VmSelfDebugPlanPayload,
    VmSelfDebugPlanResponse, VmSnapshotEntry, VmSnapshotListQuery, VmSnapshotListResponse,
    VmSnapshotPayload, VmStateStore, VmStatusResponse,
};

const VM_DIR: &str = "vm";
const VM_DISK_DIR: &str = "disks";
const VM_RUNTIME_DIR: &str = "runtime";
const VM_LOG_DIR: &str = "logs";
const VM_STATE_FILE: &str = "vm_state.json";
const VM_PROFILES_FILE: &str = "vm_exec_profiles.json";

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

fn queue_task_id() -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("q-{}-{}", now_unix(), ns)
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
    }
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
    let wd = workdir.as_deref();
    let mut out = Vec::new();
    for i in 1..=cycles {
        let start_line = format!("echo '[self-debug] cycle {i}/{cycles} start'");
        let end_line = format!("echo '[self-debug] cycle {i}/{cycles} end'");
        let cmds = [
            start_line.as_str(),
            test_cmd.as_str(),
            fix_cmd,
            verify_cmd.as_str(),
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
) -> Result<Option<VmExecResponse>, String> {
    let (task_id, cmd, timeout_sec, wait_ready_sec, ssh_user, ssh_port) = {
        let _guard = lock_recover(projects_lock, "projects_lock");
        let mut state = load_vm_state(managed_root_dir);
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
        let mut items = load_exec_queue(managed_root_dir, name);
        if items.iter().any(|x| x.status == "running") {
            return Ok(None);
        }
        let pending_idx = items
            .iter()
            .enumerate()
            .filter(|(_, x)| x.status == "pending" && now_unix() >= x.next_run_after_unix)
            .max_by(|(ia, a), (ib, b)| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| b.created_at_unix.cmp(&a.created_at_unix))
                    .then_with(|| ib.cmp(ia))
            })
            .map(|(idx, _)| idx);
        let Some(idx) = pending_idx else {
            return Ok(None);
        };
        items[idx].status = "running".to_string();
        items[idx].started_at_unix = now_unix();
        let task_id = items[idx].id.clone();
        let cmd = items[idx].command.clone();
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
            timeout_sec,
            wait_ready_sec,
            normalize_ssh_user(&vm.ssh_user),
            ssh_port,
        )
    };

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
    if let Some(item) = items.iter_mut().find(|x| x.id == task_id) {
        item.finished_at_unix = now_unix();
        item.exit_code = exec_resp.exit_code;
        item.message = exec_resp.message.clone();
        item.next_run_after_unix = 0;
        item.status = if exec_resp.exit_code == -2 {
            "canceled".to_string()
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
    }
    let _ = save_exec_queue(managed_root_dir, name, &items);
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
) {
    if !try_acquire_worker_lock(&managed_root_dir, &name) {
        return;
    }
    thread::spawn(move || {
        loop {
            match run_next_vm_exec_core(&managed_root_dir, &name, &projects_lock) {
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(e) => {
                    append_vm_log(&managed_root_dir, &name, &format!("queue worker stopped: {e}"));
                    break;
                }
            }
        }
        release_worker_lock(&managed_root_dir, &name);
    });
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
    append_vm_log(&ctx.managed_root_dir, &name, "exec task enqueued");
    spawn_vm_exec_worker(
        ctx.managed_root_dir.clone(),
        name.clone(),
        data.projects_lock.clone(),
    );
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
    let mut added = 0usize;
    for task in &body.tasks {
        if let Some(item) = normalize_batch_task(
            task,
            default_timeout_sec,
            default_wait_ready_sec,
            default_priority,
            default_retry_max,
        ) {
            items.push(item);
            added += 1;
        }
    }
    if added == 0 {
        return HttpResponse::BadRequest().body("no valid command in tasks");
    }
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("exec batch enqueued tasks={added}"),
    );
    spawn_vm_exec_worker(
        ctx.managed_root_dir.clone(),
        name.clone(),
        data.projects_lock.clone(),
    );
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
    let mut added = 0usize;
    for task in &tasks {
        if let Some(item) = normalize_batch_task(
            task,
            default_timeout_sec,
            default_wait_ready_sec,
            default_priority,
            default_retry_max,
        ) {
            items.push(item);
            added += 1;
        }
    }
    if added == 0 {
        return HttpResponse::BadRequest().body("profile has no runnable command");
    }
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("exec profile enqueued profile={profile} tasks={added}"),
    );
    spawn_vm_exec_worker(
        ctx.managed_root_dir.clone(),
        name.clone(),
        data.projects_lock.clone(),
    );
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
    let tasks = match build_self_debug_tasks(&body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    HttpResponse::Ok().json(VmSelfDebugPlanResponse {
        name,
        total_tasks: tasks.len(),
        tasks: tasks.into_iter().map(|x| x.command).collect(),
        message: "self-debug plan generated".to_string(),
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
    let tasks = match build_self_debug_tasks(&body) {
        Ok(v) => v,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let default_timeout_sec = if body.timeout_sec == 0 {
        120
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
    let mut added = 0usize;
    for task in &tasks {
        if let Some(item) = normalize_batch_task(
            task,
            default_timeout_sec,
            default_wait_ready_sec,
            default_priority,
            default_retry_max,
        ) {
            items.push(item);
            added += 1;
        }
    }
    if added == 0 {
        return HttpResponse::BadRequest().body("self-debug plan has no runnable command");
    }
    if let Err(e) = save_exec_queue(&ctx.managed_root_dir, &name, &items) {
        return HttpResponse::InternalServerError().body(format!("save queue failed: {e}"));
    }
    append_vm_log(
        &ctx.managed_root_dir,
        &name,
        &format!("self-debug plan enqueued tasks={added}"),
    );
    spawn_vm_exec_worker(
        ctx.managed_root_dir.clone(),
        name.clone(),
        data.projects_lock.clone(),
    );
    HttpResponse::Ok().json(VmSelfDebugPlanResponse {
        name,
        total_tasks: tasks.len(),
        tasks: tasks.into_iter().map(|x| x.command).collect(),
        message: "self-debug plan enqueued".to_string(),
    })
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

    match run_next_vm_exec_core(&ctx.managed_root_dir, &name, &data.projects_lock) {
        Ok(Some(resp)) => HttpResponse::Ok().json(resp),
        Ok(None) => HttpResponse::BadRequest().body("no runnable queued task"),
        Err(e) => HttpResponse::BadRequest().body(e),
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
    let Some(vm) = state.vms.get_mut(&name) else {
        return HttpResponse::NotFound().body("vm not found");
    };

    reconcile_vm_power_state(&ctx.managed_root_dir, vm);
    if vm.power_state == "running" {
        let snapshot = vm.clone();
        append_vm_log(
            &ctx.managed_root_dir,
            &snapshot.name,
            "start requested but vm already running",
        );
        return HttpResponse::Ok().json(VmActionResponse {
            ok: true,
            effective: true,
            message: "vm already running".to_string(),
            vm: Some(snapshot),
        });
    }

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
    if effective {
        spawn_vm_exec_worker(
            ctx.managed_root_dir.clone(),
            name.clone(),
            data.projects_lock.clone(),
        );
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
    use super::{default_cpu, default_disk_gb, default_memory_mb, normalize_backend, sanitize_vm_name};

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
}

(() => {
function assertVmDependencies() {
    const root = window.KACF || {};
    const state = root.state || {};
    const logPipeline = root.logPipeline || {};
    const requiredState = ['txt', 'fmt', 'isReadOnlyView'];
    const requiredLog = ['setStatus', 'appendLog'];
    requiredState.forEach((name) => {
        if (typeof state[name] !== 'function') {
            throw new Error(`KACF.state.${name} is not available`);
        }
    });
    requiredLog.forEach((name) => {
        if (typeof logPipeline[name] !== 'function') {
            throw new Error(`KACF.logPipeline.${name} is not available`);
        }
    });
}

assertVmDependencies();

const { txt, fmt, isReadOnlyView } = window.KACF.state;
const { setStatus, appendLog } = window.KACF.logPipeline;

let vmReadonly = false;
let vmGuestMode = false;
let vmBound = false;
let vmRefreshTimer = null;
let vmLogRefreshTimer = null;
let vmQueueRefreshTimer = null;
let vmExecProfiles = [];

function el(id) {
    return document.getElementById(id);
}

function asPositiveInt(raw, fallback) {
    const n = Number.parseInt(String(raw || '').trim(), 10);
    if (!Number.isFinite(n) || n <= 0) return fallback;
    return n;
}

function asBoundedInt(raw, min, max, fallback) {
    const n = Number.parseInt(String(raw || '').trim(), 10);
    if (!Number.isFinite(n)) return fallback;
    if (n < min) return min;
    if (n > max) return max;
    return n;
}

function asBoundedSignedInt(raw, min, max, fallback) {
    const n = Number.parseInt(String(raw || '').trim(), 10);
    if (!Number.isFinite(n)) return fallback;
    if (n < min) return min;
    if (n > max) return max;
    return n;
}

function selectedVmName() {
    return (el('vm_target_select')?.value || '').trim();
}

function setVmStatusLine(text) {
    const node = el('vm_status_text');
    if (node) node.textContent = text || '';
}

function setVmLogsText(text) {
    const node = el('vm_logs');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSnapshotsText(text) {
    const node = el('vm_snapshots');
    if (!node) return;
    node.textContent = text || '';
}

function setVmExecOutput(text) {
    const node = el('vm_exec_output');
    if (!node) return;
    node.textContent = text || '';
}

function setVmExecQueueText(text) {
    const node = el('vm_exec_queue');
    if (!node) return;
    node.textContent = text || '';
}

function setVmExecQueueStatsText(text) {
    const node = el('vm_exec_queue_stats');
    if (!node) return;
    node.textContent = text || '';
}

function setVmExecProfilePreviewText(text) {
    const node = el('vm_exec_profile_preview');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSelfDebugPlanText(text) {
    const node = el('vm_self_debug_plan');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSelfDebugRunsText(text) {
    const node = el('vm_self_debug_runs');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSelfDebugHistoryText(text) {
    const node = el('vm_self_debug_history');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSelfDebugRunDetailText(text) {
    const node = el('vm_self_debug_run_detail');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSelfDebugStrategyStatsText(text) {
    const node = el('vm_self_debug_strategy_stats');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSelfDebugTimelineText(text) {
    const node = el('vm_self_debug_timeline');
    if (!node) return;
    node.textContent = text || '';
}

function setVmSelfDebugContextText(text) {
    const node = el('vm_self_debug_context');
    if (!node) return;
    node.textContent = text || '';
}

function buildSelfDebugTimelineText(tasks) {
    const list = Array.isArray(tasks) ? tasks : [];
    if (!list.length) {
        return txt('vm_self_debug_timeline_empty', 'No self-debug timeline.');
    }
    const byTrigger = new Map();
    const failures = [];
    const others = [];
    list.forEach((t) => {
        const kind = String(t?.run_kind || '');
        const id = String(t?.id || '');
        if (kind === 'self_debug' && String(t?.status || '') === 'failed') {
            failures.push(t);
            return;
        }
        if (kind === 'self_debug_strategy' || kind === 'self_debug_verify_after_strategy') {
            const trigger = String(t?.trigger_task_id || '');
            if (!byTrigger.has(trigger)) byTrigger.set(trigger, []);
            byTrigger.get(trigger).push(t);
            return;
        }
        others.push(t);
    });
    const lines = [];
    lines.push(txt('vm_self_debug_timeline_title', 'Self-Debug Timeline'));
    if (failures.length) {
        failures.forEach((f, i) => {
            const fid = String(f?.id || '-');
            const fcat = String(f?.failure_category || '-');
            const fsig = String(f?.failure_signature || '-');
            lines.push(`#${i + 1} [Failure] ${fid} cat=${fcat} sig=${fsig}`);
            const branch = byTrigger.get(fid) || [];
            const strategies = branch.filter((x) => String(x?.run_kind || '') === 'self_debug_strategy');
            const verifiers = branch.filter((x) => String(x?.run_kind || '') === 'self_debug_verify_after_strategy');
            if (!strategies.length && !verifiers.length) {
                lines.push(`  -> ${txt('vm_self_debug_timeline_no_strategy', 'No strategy branch')}`);
            }
            strategies.forEach((s) => {
                lines.push(`  -> [Strategy] ${String(s?.id || '-')} [${String(s?.status || '-')}]`);
            });
            verifiers.forEach((v) => {
                lines.push(`  -> [VerifyAfter] ${String(v?.id || '-')} [${String(v?.status || '-')}]`);
            });
        });
    } else {
        lines.push(txt('vm_self_debug_timeline_no_failure', 'No failed self-debug step in this run.'));
    }
    if (others.length) {
        lines.push(txt('vm_self_debug_timeline_other_title', 'Other Steps:'));
        others.slice(0, 10).forEach((x) => {
            lines.push(`  - ${String(x?.id || '-')} kind=${String(x?.run_kind || '-')} [${String(x?.status || '-')}]`);
        });
    }
    return lines.join('\n');
}

function setVmMutatingDisabled(disabled) {
    [
        'vm_provision_btn',
        'vm_start_btn',
        'vm_stop_btn',
        'vm_delete_btn',
        'vm_snapshot_create_btn',
        'vm_snapshot_apply_btn',
        'vm_snapshot_delete_btn',
        'vm_clone_btn',
        'vm_exec_btn',
        'vm_bootstrap_btn',
        'vm_exec_cancel_btn',
        'vm_exec_enqueue_btn',
        'vm_exec_batch_enqueue_btn',
        'vm_exec_profile_enqueue_btn',
        'vm_exec_profile_preview_btn',
        'vm_exec_custom_profile_load_btn',
        'vm_exec_custom_profile_save_btn',
        'vm_exec_custom_profile_delete_btn',
        'vm_self_debug_preview_btn',
        'vm_self_debug_start_btn',
        'vm_self_debug_runs_refresh_btn',
        'vm_self_debug_history_refresh_btn',
        'vm_self_debug_history_detail_btn',
        'vm_self_debug_history_archive_btn',
        'vm_self_debug_history_clear_btn',
        'vm_self_debug_stats_btn',
        'vm_self_debug_detail_btn',
        'vm_self_debug_context_btn',
        'vm_self_debug_apply_context_btn',
        'vm_self_debug_pause_btn',
        'vm_self_debug_resume_btn',
        'vm_self_debug_stop_btn',
        'vm_self_debug_rules_load_btn',
        'vm_self_debug_rules_save_btn',
        'vm_exec_run_next_btn',
        'vm_exec_queue_refresh_btn',
        'vm_exec_queue_cancel_btn',
        'vm_exec_profile',
        'vm_exec_custom_profile_name',
        'vm_exec_custom_profile_commands',
        'vm_self_debug_cycles',
        'vm_self_debug_fix_cmd',
        'vm_self_debug_verify_cmd',
        'vm_self_debug_success_streak_target',
        'vm_self_debug_fail_streak_target',
        'vm_self_debug_max_task_budget',
        'vm_self_debug_max_runtime_sec',
        'vm_self_debug_run_id',
        'vm_self_debug_strategy_rules',
    ].forEach((id) => {
        const node = el(id);
        if (node) node.disabled = !!disabled;
    });
}

function setVmReadOnlyByContext() {
    const readonly = vmGuestMode || vmReadonly || isReadOnlyView();
    setVmMutatingDisabled(readonly);
}

function renderCapabilities(caps) {
    const list = Array.isArray(caps) ? caps : [];
    const text = list.map((x) => `${x.backend}: ${x.available ? 'OK' : 'N/A'} (${x.detail || '-'})`).join(' | ');
    const node = el('vm_capabilities');
    if (node) {
        node.textContent = fmt('vm_capabilities_line', 'Capabilities: {caps}', {
            caps: text || txt('none_text', 'None'),
        });
    }
}

function renderVmTargetList(vms) {
    const select = el('vm_target_select');
    if (!select) return;
    const items = Array.isArray(vms) ? vms : [];
    const current = select.value;
    select.innerHTML = '';
    if (!items.length) {
        const opt = document.createElement('option');
        opt.value = '';
        opt.textContent = txt('vm_target_empty', 'No VM available');
        select.appendChild(opt);
        return;
    }
    items.forEach((vm) => {
        const opt = document.createElement('option');
        opt.value = vm.name || '';
        opt.textContent = `${vm.name || '-'} (${vm.power_state || '-'})`;
        select.appendChild(opt);
    });
    if (current && items.some((vm) => vm.name === current)) {
        select.value = current;
    }
}

function renderVmList(vms) {
    const items = Array.isArray(vms) ? vms : [];
    const node = el('vm_list');
    if (!node) return;
    if (!items.length) {
        node.textContent = txt('vm_list_empty', 'No VM instance yet.');
        return;
    }
    const lines = [];
    items.forEach((vm, idx) => {
        lines.push(`#${idx + 1} ${vm.name || '-'}`);
        lines.push(`  state=${vm.power_state || '-'} backend=${vm.backend || '-'} cpu=${vm.cpu || '-'} mem=${vm.memory_mb || '-'}MB disk=${vm.disk_gb || '-'}GB`);
        lines.push(`  ssh=${vm.ssh_user || 'root'}@127.0.0.1:${vm.ssh_port || '-'} pid=${vm.process_id || '-'}`);
        lines.push(`  disk=${vm.disk_path || '-'} image=${vm.os_image || '-'}`);
        lines.push(`  msg=${vm.last_message || '-'}`);
    });
    node.textContent = lines.join('\n');
}

async function vmApi(path, method, payload) {
    const resp = await fetch(path, {
        method,
        headers: { 'Content-Type': 'application/json' },
        body: payload ? JSON.stringify(payload) : undefined,
    });
    if (!resp.ok) {
        const msg = await resp.text();
        throw new Error(`${resp.status} ${msg}`);
    }
    return resp.json();
}

async function refreshVmStatus() {
    try {
        const data = await vmApi('/vm/status', 'GET');
        vmReadonly = !!data.readonly;
        setVmReadOnlyByContext();
        renderCapabilities(data.capabilities || []);
        renderVmTargetList(data.vms || []);
        renderVmList(data.vms || []);
        renderVmExecProfiles();
        setVmStatusLine(txt('status_vm_synced', 'VM status synchronized'));
        await refreshVmSnapshots();
        await refreshVmLogs();
    } catch (e) {
        setVmStatusLine(fmt('status_vm_sync_failed', 'VM status sync failed: {error}', { error: String(e) }));
    }
}

async function refreshVmSnapshots() {
    const name = selectedVmName();
    if (!name) {
        setVmSnapshotsText(txt('vm_snapshot_empty', 'No snapshots yet.'));
        return;
    }
    try {
        const data = await vmApi(`/vm/snapshot/list?name=${encodeURIComponent(name)}`, 'GET');
        const list = Array.isArray(data?.snapshots) ? data.snapshots : [];
        if (!list.length) {
            setVmSnapshotsText(txt('vm_snapshot_empty', 'No snapshots yet.'));
            return;
        }
        const lines = list.map((s, i) => {
            const tag = String(s?.tag || '-');
            const size = String(s?.vm_size || '-');
            const at = String(s?.created_at || '-');
            const clk = String(s?.vm_clock || '-');
            return `#${i + 1} ${tag} | size=${size} | at=${at} | clock=${clk}`;
        });
        setVmSnapshotsText(lines.join('\n'));
    } catch (e) {
        setVmSnapshotsText(fmt('status_vm_snapshot_list_failed', 'List VM snapshots failed: {error}', { error: String(e) }));
    }
}

function snapshotNameInput() {
    return (el('vm_snapshot_name')?.value || '').trim();
}

function cloneNewNameInput() {
    return (el('vm_clone_new_name')?.value || '').trim();
}

function profileWorkdirInput() {
    return String(el('vm_exec_profile_workdir')?.value || '').trim();
}

function profileTestCmdInput() {
    return String(el('vm_exec_profile_test_cmd')?.value || '').trim();
}

function collectVmSelfDebugPayload() {
    return {
        name: selectedVmName(),
        run_id: String(el('vm_self_debug_run_id')?.value || '').trim(),
        profile: String(el('vm_exec_profile')?.value || '').trim(),
        cycles: asBoundedInt(el('vm_self_debug_cycles')?.value, 1, 30, 3),
        workdir: profileWorkdirInput(),
        test_cmd: profileTestCmdInput(),
        fix_cmd: String(el('vm_self_debug_fix_cmd')?.value || '').trim(),
        verify_cmd: String(el('vm_self_debug_verify_cmd')?.value || '').trim(),
        success_streak_target: asBoundedInt(el('vm_self_debug_success_streak_target')?.value, 1, 10, 2),
        fail_streak_target: asBoundedInt(el('vm_self_debug_fail_streak_target')?.value, 1, 10, 3),
        max_task_budget: asBoundedInt(el('vm_self_debug_max_task_budget')?.value, 0, 500, 0),
        max_runtime_sec: asBoundedInt(el('vm_self_debug_max_runtime_sec')?.value, 0, 86400, 0),
        timeout_sec: asBoundedInt(el('vm_exec_timeout_sec')?.value, 1, 3600, 120),
        wait_ready_sec: asBoundedInt(el('vm_exec_wait_ready_sec')?.value, 0, 600, 0),
        priority: asBoundedSignedInt(el('vm_exec_priority')?.value, -100, 100, 0),
        retry_max: asBoundedInt(el('vm_exec_retry_max')?.value, 0, 10, 0),
    };
}

async function createVmSnapshot() {
    const name = selectedVmName();
    const snapshot = snapshotNameInput();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!snapshot) {
        setStatus(txt('status_vm_snapshot_name_required', 'Please enter snapshot name first.'), 'status-danger');
        return;
    }
    try {
        const res = await vmApi('/vm/snapshot/create', 'POST', { name, snapshot });
        setStatus(txt('status_vm_snapshot_create_ok', 'Snapshot created.'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'snapshot create ok' }));
        await refreshVmStatus();
        await refreshVmSnapshots();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_snapshot_create_failed', 'Snapshot create failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function applyVmSnapshot() {
    const name = selectedVmName();
    const snapshot = snapshotNameInput();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!snapshot) {
        setStatus(txt('status_vm_snapshot_name_required', 'Please enter snapshot name first.'), 'status-danger');
        return;
    }
    const confirmText = fmt('confirm_vm_snapshot_apply', 'Apply snapshot "{snapshot}" to VM "{name}"?', { name, snapshot });
    if (!window.confirm(confirmText)) return;
    try {
        const res = await vmApi('/vm/snapshot/apply', 'POST', { name, snapshot });
        setStatus(txt('status_vm_snapshot_apply_ok', 'Snapshot applied.'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'snapshot apply ok' }));
        await refreshVmStatus();
        await refreshVmSnapshots();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_snapshot_apply_failed', 'Snapshot apply failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function deleteVmSnapshot() {
    const name = selectedVmName();
    const snapshot = snapshotNameInput();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!snapshot) {
        setStatus(txt('status_vm_snapshot_name_required', 'Please enter snapshot name first.'), 'status-danger');
        return;
    }
    const confirmText = fmt('confirm_vm_snapshot_delete', 'Delete snapshot "{snapshot}" from VM "{name}"?', { name, snapshot });
    if (!window.confirm(confirmText)) return;
    try {
        const res = await vmApi('/vm/snapshot/delete', 'POST', { name, snapshot });
        setStatus(txt('status_vm_snapshot_delete_ok', 'Snapshot deleted.'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'snapshot delete ok' }));
        await refreshVmStatus();
        await refreshVmSnapshots();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_snapshot_delete_failed', 'Snapshot delete failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function cloneVmFromSnapshot() {
    const source_name = selectedVmName();
    const snapshot = snapshotNameInput();
    const new_name = cloneNewNameInput();
    if (!source_name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!new_name) {
        setStatus(txt('status_vm_clone_name_required', 'Please enter new VM name first.'), 'status-danger');
        return;
    }
    const confirmText = fmt('confirm_vm_clone', 'Clone VM "{source}" into "{target}"?', {
        source: source_name,
        target: new_name,
    });
    if (!window.confirm(confirmText)) return;
    try {
        const res = await vmApi('/vm/clone', 'POST', { source_name, new_name, snapshot });
        setStatus(txt('status_vm_clone_ok', 'VM clone created.'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'clone ok' }));
        await refreshVmStatus();
        await refreshVmSnapshots();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_clone_failed', 'VM clone failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function execInVm() {
    const name = selectedVmName();
    const command = (el('vm_exec_command')?.value || '').trim();
    const timeoutSec = asBoundedInt(el('vm_exec_timeout_sec')?.value, 1, 3600, 60);
    const waitReadySec = asBoundedInt(el('vm_exec_wait_ready_sec')?.value, 0, 600, 0);
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!command) {
        setStatus(txt('status_vm_exec_command_required', 'Please enter VM command first.'), 'status-danger');
        return;
    }
    setVmExecOutput('');
    try {
        const res = await vmApi('/vm/exec', 'POST', {
            name,
            command,
            timeout_sec: timeoutSec,
            wait_ready_sec: waitReadySec,
        });
        const lines = [];
        lines.push(`ok=${!!res.ok} exit=${Number(res.exit_code)}`);
        lines.push(`message=${res.message || ''}`);
        lines.push('--- stdout ---');
        lines.push(String(res.stdout || ''));
        lines.push('--- stderr ---');
        lines.push(String(res.stderr || ''));
        setVmExecOutput(lines.join('\n'));
        if (res.ok) {
            setStatus(txt('status_vm_exec_ok', 'VM command executed.'), 'status-warn');
        } else {
            setStatus(txt('status_vm_exec_nonzero', 'VM command finished with non-zero exit.'), 'status-danger');
        }
        await refreshVmStatus();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_exec_failed', 'VM exec failed: {error}', { error: String(e) }), 'status-danger');
        setVmExecOutput(String(e));
    }
}

async function enqueueVmExec() {
    const name = selectedVmName();
    const command = (el('vm_exec_command')?.value || '').trim();
    const timeoutSec = asBoundedInt(el('vm_exec_timeout_sec')?.value, 1, 3600, 60);
    const waitReadySec = asBoundedInt(el('vm_exec_wait_ready_sec')?.value, 0, 600, 0);
    const priority = asBoundedSignedInt(el('vm_exec_priority')?.value, -100, 100, 0);
    const retryMax = asBoundedInt(el('vm_exec_retry_max')?.value, 0, 10, 0);
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!command) {
        setStatus(txt('status_vm_exec_command_required', 'Please enter VM command first.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/exec/enqueue', 'POST', {
            name,
            command,
            timeout_sec: timeoutSec,
            wait_ready_sec: waitReadySec,
            priority,
            retry_max: retryMax,
        });
        setStatus(txt('status_vm_exec_enqueue_ok', 'VM command queued.'), 'status-warn');
        await refreshVmExecQueue();
    } catch (e) {
        setStatus(fmt('status_vm_exec_queue_failed', 'VM exec queue operation failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function enqueueVmExecBatch() {
    const name = selectedVmName();
    const timeoutSec = asBoundedInt(el('vm_exec_timeout_sec')?.value, 1, 3600, 60);
    const waitReadySec = asBoundedInt(el('vm_exec_wait_ready_sec')?.value, 0, 600, 0);
    const priority = asBoundedSignedInt(el('vm_exec_priority')?.value, -100, 100, 0);
    const retryMax = asBoundedInt(el('vm_exec_retry_max')?.value, 0, 10, 0);
    const raw = String(el('vm_exec_batch_commands')?.value || '');
    const tasks = raw
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean)
        .map((command) => ({ command }));
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!tasks.length) {
        setStatus(txt('status_vm_exec_batch_required', 'Please enter at least one command in batch list.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/exec/enqueue_batch', 'POST', {
            name,
            timeout_sec: timeoutSec,
            wait_ready_sec: waitReadySec,
            priority,
            retry_max: retryMax,
            tasks,
        });
        setStatus(fmt('status_vm_exec_batch_ok', 'Queued {count} VM commands.', { count: tasks.length }), 'status-warn');
        await refreshVmExecQueue();
    } catch (e) {
        setStatus(fmt('status_vm_exec_queue_failed', 'VM exec queue operation failed: {error}', { error: String(e) }), 'status-danger');
    }
}

function vmExecProfileLabel(profile) {
    const key = `vm_exec_profile_${String(profile || '').replace(/-/g, '_')}`;
    return txt(key, String(profile || ''));
}

function isCustomProfileName(name) {
    return /^custom-[a-z0-9_-]{1,56}$/.test(String(name || '').trim());
}

function renderVmExecProfiles() {
    const select = el('vm_exec_profile');
    if (!select) return;
    const current = String(select.value || '').trim();
    select.innerHTML = '';
    const list = Array.isArray(vmExecProfiles) ? vmExecProfiles : [];
    if (!list.length) {
        const opt = document.createElement('option');
        opt.value = '';
        opt.textContent = txt('vm_exec_profile_empty', 'No profile available');
        select.appendChild(opt);
        return;
    }
    list.forEach((profile) => {
        const p = String(profile || '').trim();
        if (!p) return;
        const opt = document.createElement('option');
        opt.value = p;
        opt.textContent = vmExecProfileLabel(p);
        select.appendChild(opt);
    });
    if (current && list.includes(current)) {
        select.value = current;
    } else if (list.length) {
        select.value = list[0];
    }
}

async function refreshVmExecProfiles() {
    try {
        const data = await vmApi('/vm/exec/profiles', 'GET');
        vmExecProfiles = Array.isArray(data?.profiles) ? data.profiles.map((x) => String(x || '').trim()).filter(Boolean) : [];
        renderVmExecProfiles();
        await previewVmExecProfile();
    } catch (_e) {
        vmExecProfiles = [];
        renderVmExecProfiles();
        setVmExecProfilePreviewText(txt('vm_exec_profile_preview_empty', 'No preview available.'));
    }
}

async function loadSelectedProfileToEditor() {
    const profile = String(el('vm_exec_profile')?.value || '').trim();
    if (!profile) {
        setStatus(txt('status_vm_exec_profile_required', 'Please select an exec profile first.'), 'status-danger');
        return;
    }
    try {
        const data = await vmApi(`/vm/exec/profile/detail?profile=${encodeURIComponent(profile)}`, 'GET');
        const commands = Array.isArray(data?.commands) ? data.commands : [];
        const nameInput = el('vm_exec_custom_profile_name');
        const cmdArea = el('vm_exec_custom_profile_commands');
        if (nameInput) {
            nameInput.value = isCustomProfileName(profile) ? profile : '';
        }
        if (cmdArea) {
            cmdArea.value = commands.map((x) => String(x || '').trim()).filter(Boolean).join('\n');
        }
        setStatus(txt('status_vm_profile_loaded_to_editor', 'Profile commands loaded to editor.'), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_exec_profile_load_failed', 'Load profile detail failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function saveCustomProfile() {
    const name = String(el('vm_exec_custom_profile_name')?.value || '').trim();
    const raw = String(el('vm_exec_custom_profile_commands')?.value || '');
    const commands = raw.split('\n').map((s) => s.trim()).filter(Boolean);
    if (!isCustomProfileName(name)) {
        setStatus(txt('status_vm_custom_profile_name_invalid', 'Custom profile name must start with custom- and use lowercase letters, digits, dash or underscore.'), 'status-danger');
        return;
    }
    if (!commands.length) {
        setStatus(txt('status_vm_custom_profile_commands_required', 'Please enter at least one command for custom profile.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/exec/profiles/save', 'POST', { name, commands });
        await refreshVmExecProfiles();
        const select = el('vm_exec_profile');
        if (select) select.value = name;
        await previewVmExecProfile();
        setStatus(fmt('status_vm_custom_profile_saved', 'Custom profile saved: {name}', { name }), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_custom_profile_save_failed', 'Save custom profile failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function deleteCustomProfile() {
    const name = String(el('vm_exec_custom_profile_name')?.value || '').trim();
    if (!isCustomProfileName(name)) {
        setStatus(txt('status_vm_custom_profile_name_invalid', 'Custom profile name must start with custom- and use lowercase letters, digits, dash or underscore.'), 'status-danger');
        return;
    }
    const confirmText = fmt('confirm_vm_custom_profile_delete', 'Delete custom profile "{name}"?', { name });
    if (!window.confirm(confirmText)) return;
    try {
        await vmApi('/vm/exec/profiles/delete', 'POST', { name });
        const nameInput = el('vm_exec_custom_profile_name');
        const cmdArea = el('vm_exec_custom_profile_commands');
        if (nameInput) nameInput.value = '';
        if (cmdArea) cmdArea.value = '';
        await refreshVmExecProfiles();
        await previewVmExecProfile();
        setStatus(fmt('status_vm_custom_profile_deleted', 'Custom profile deleted: {name}', { name }), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_custom_profile_delete_failed', 'Delete custom profile failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function previewVmSelfDebugPlan() {
    const payload = collectVmSelfDebugPayload();
    if (!payload.name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!payload.fix_cmd) {
        setStatus(txt('status_vm_self_debug_fix_required', 'Please enter fix command for self-debug plan.'), 'status-danger');
        return;
    }
    try {
        const data = await vmApi('/vm/self_debug/plan', 'POST', payload);
        const list = Array.isArray(data?.tasks) ? data.tasks : [];
        const lines = list.map((cmd, i) => `#${i + 1} ${String(cmd || '')}`);
        setVmSelfDebugPlanText(lines.join('\n'));
        setStatus(fmt('status_vm_self_debug_plan_ready', 'Self-debug plan ready ({count} tasks).', { count: list.length }), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_plan_failed', 'Self-debug plan failed: {error}', { error: String(e) }), 'status-danger');
        setVmSelfDebugPlanText(String(e));
    }
}

async function startVmSelfDebugPlan() {
    const payload = collectVmSelfDebugPayload();
    if (!payload.name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!payload.fix_cmd) {
        setStatus(txt('status_vm_self_debug_fix_required', 'Please enter fix command for self-debug plan.'), 'status-danger');
        return;
    }
    try {
        const data = await vmApi('/vm/self_debug/start', 'POST', payload);
        const list = Array.isArray(data?.tasks) ? data.tasks : [];
        const lines = list.map((cmd, i) => `#${i + 1} ${String(cmd || '')}`);
        setVmSelfDebugPlanText(lines.join('\n'));
        setStatus(fmt('status_vm_self_debug_started', 'Self-debug plan queued ({count} tasks).', { count: list.length }), 'status-warn');
        const runIdInput = el('vm_self_debug_run_id');
        if (runIdInput && data?.run_id) runIdInput.value = String(data.run_id);
        await refreshVmExecQueue();
        await refreshVmSelfDebugRuns();
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_start_failed', 'Start self-debug plan failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function refreshVmSelfDebugRuns() {
    const name = selectedVmName();
    if (!name) {
        setVmSelfDebugRunsText('');
        return;
    }
    try {
        const data = await vmApi(`/vm/self_debug/runs?name=${encodeURIComponent(name)}`, 'GET');
        const runs = Array.isArray(data?.runs) ? data.runs : [];
        if (!runs.length) {
            setVmSelfDebugRunsText(txt('vm_self_debug_runs_empty', 'No self-debug runs.'));
            return;
        }
        const lines = runs.map((r, i) => {
            const id = String(r?.run_id || '-');
            const total = Number(r?.total || 0);
            const pending = Number(r?.pending || 0);
            const paused = Number(r?.paused || 0);
            const running = Number(r?.running || 0);
            const done = Number(r?.done || 0);
            const failed = Number(r?.failed || 0);
            const canceled = Number(r?.canceled || 0);
            const at = Number(r?.updated_at_unix || 0);
            return `#${i + 1} ${id} total=${total} p=${pending} z=${paused} r=${running} d=${done} f=${failed} c=${canceled} at=${at}`;
        });
        setVmSelfDebugRunsText(lines.join('\n'));
    } catch (e) {
        setVmSelfDebugRunsText(fmt('status_vm_self_debug_runs_failed', 'Load self-debug runs failed: {error}', { error: String(e) }));
    }
}

async function refreshVmSelfDebugHistory() {
    const name = selectedVmName();
    if (!name) {
        setVmSelfDebugHistoryText('');
        return;
    }
    try {
        const data = await vmApi(`/vm/self_debug/history?name=${encodeURIComponent(name)}`, 'GET');
        const history = Array.isArray(data?.history) ? data.history : [];
        if (!history.length) {
            setVmSelfDebugHistoryText(txt('vm_self_debug_history_empty', 'No self-debug history.'));
            return;
        }
        const lines = history.map((h, i) => {
            const s = h?.summary || {};
            const id = String(s?.run_id || '-');
            const total = Number(s?.total || 0);
            const done = Number(s?.done || 0);
            const failed = Number(s?.failed || 0);
            const canceled = Number(s?.canceled || 0);
            const at = Number(s?.updated_at_unix || 0);
            const archivedAt = Number(h?.archived_at_unix || 0);
            return `#${i + 1} ${id} total=${total} d=${done} f=${failed} c=${canceled} at=${at} archived=${archivedAt}`;
        });
        setVmSelfDebugHistoryText(lines.join('\n'));
    } catch (e) {
        setVmSelfDebugHistoryText(fmt('status_vm_self_debug_history_failed', 'Load self-debug history failed: {error}', { error: String(e) }));
    }
}

async function loadVmSelfDebugHistoryDetail() {
    const name = selectedVmName();
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!runId) {
        setStatus(txt('status_vm_self_debug_run_id_required', 'Please enter self-debug run id first.'), 'status-danger');
        return;
    }
    try {
        const data = await vmApi(`/vm/self_debug/history/detail?name=${encodeURIComponent(name)}&run_id=${encodeURIComponent(runId)}`, 'GET');
        const entry = data?.entry || {};
        const summary = entry?.summary || {};
        const archivedAt = Number(entry?.archived_at_unix || 0);
        const failedSteps = Number(entry?.failed_steps || 0);
        const categories = Array.isArray(entry?.categories) ? entry.categories : [];
        const keyLines = Array.isArray(entry?.key_lines) ? entry.key_lines : [];
        const contextText = String(entry?.context_text || '').trim();
        const lines = [];
        lines.push(`run_id=${String(summary?.run_id || runId)}`);
        lines.push(`archived_at=${archivedAt}`);
        lines.push(`total=${Number(summary?.total || 0)} done=${Number(summary?.done || 0)} failed=${Number(summary?.failed || 0)} canceled=${Number(summary?.canceled || 0)}`);
        lines.push(`failed_steps=${failedSteps}`);
        lines.push(`categories=${categories.join(', ')}`);
        if (keyLines.length) {
            lines.push(`key_lines=${keyLines.join(' | ')}`);
        }
        if (contextText) {
            lines.push('');
            lines.push(contextText);
        }
        setVmSelfDebugContextText(lines.join('\n'));
        setStatus(txt('status_vm_self_debug_history_detail_loaded', 'Self-debug history detail loaded.'), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_history_detail_failed', 'Load self-debug history detail failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function archiveVmSelfDebugCompletedRuns() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    try {
        const data = await vmApi('/vm/self_debug/history/archive_completed', 'POST', { name });
        const archived = Number(data?.archived_runs || 0);
        const removed = Number(data?.removed_tasks || 0);
        setStatus(
            fmt('status_vm_self_debug_history_archived', 'Self-debug history archived runs={runs}, removed tasks={tasks}.', {
                runs: archived,
                tasks: removed,
            }),
            'status-warn'
        );
        await refreshVmExecQueue();
        await refreshVmSelfDebugRuns();
        await refreshVmSelfDebugHistory();
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_history_archive_failed', 'Archive self-debug history failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function clearVmSelfDebugHistory() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    const confirmText = runId
        ? fmt('confirm_vm_self_debug_history_clear_one', 'Clear self-debug history for run "{run_id}"?', { run_id: runId })
        : txt('confirm_vm_self_debug_history_clear_all', 'Clear all self-debug history?');
    if (!window.confirm(confirmText)) return;
    try {
        const data = await vmApi('/vm/self_debug/history/clear', 'POST', { name, run_id: runId });
        const removed = Number(data?.removed || 0);
        setStatus(fmt('status_vm_self_debug_history_cleared', 'Self-debug history cleared, removed={count}.', { count: removed }), 'status-warn');
        await refreshVmSelfDebugHistory();
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_history_clear_failed', 'Clear self-debug history failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function refreshVmSelfDebugStrategyStats() {
    const name = selectedVmName();
    if (!name) {
        setVmSelfDebugStrategyStatsText('');
        return;
    }
    try {
        const data = await vmApi(`/vm/self_debug/strategy_stats?name=${encodeURIComponent(name)}`, 'GET');
        const stats = Array.isArray(data?.stats) ? data.stats : [];
        if (!stats.length) {
            setVmSelfDebugStrategyStatsText(txt('vm_self_debug_strategy_stats_empty', 'No strategy stats yet.'));
            return;
        }
        const lines = stats.map((s, i) => {
            const cat = String(s?.category || '-');
            const attempts = Number(s?.attempts || 0);
            const ok = Number(s?.verified_success || 0);
            const fail = Number(s?.verified_fail || 0);
            const pending = Number(s?.pending || 0);
            const rate = Number(s?.success_rate || 0).toFixed(1);
            return `#${i + 1} cat=${cat} attempts=${attempts} ok=${ok} fail=${fail} pending=${pending} success=${rate}%`;
        });
        setVmSelfDebugStrategyStatsText(lines.join('\n'));
    } catch (e) {
        setVmSelfDebugStrategyStatsText(fmt('status_vm_self_debug_strategy_stats_failed', 'Load self-debug strategy stats failed: {error}', { error: String(e) }));
    }
}

function formatStrategyRulesText(rulesObj) {
    const obj = rulesObj && typeof rulesObj === 'object' ? rulesObj : {};
    return JSON.stringify(obj, null, 2);
}

function parseStrategyRulesText(raw) {
    const text = String(raw || '').trim();
    if (!text) return {};
    const parsed = JSON.parse(text);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
        throw new Error('rules must be a JSON object');
    }
    return parsed;
}

async function loadVmSelfDebugStrategyRules() {
    try {
        const data = await vmApi('/vm/self_debug/strategy_rules', 'GET');
        const area = el('vm_self_debug_strategy_rules');
        if (area) area.value = formatStrategyRulesText(data?.rules || {});
        setStatus(txt('status_vm_self_debug_rules_loaded', 'Strategy rules loaded.'), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_rules_load_failed', 'Load strategy rules failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function saveVmSelfDebugStrategyRules() {
    try {
        const area = el('vm_self_debug_strategy_rules');
        const rules = parseStrategyRulesText(area?.value || '');
        await vmApi('/vm/self_debug/strategy_rules', 'POST', { rules });
        setStatus(txt('status_vm_self_debug_rules_saved', 'Strategy rules saved.'), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_rules_save_failed', 'Save strategy rules failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function refreshVmSelfDebugRunDetail() {
    const name = selectedVmName();
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    if (!name || !runId) {
        setVmSelfDebugTimelineText('');
        setVmSelfDebugRunDetailText('');
        setVmSelfDebugContextText('');
        return;
    }
    try {
        const data = await vmApi(`/vm/self_debug/run_detail?name=${encodeURIComponent(name)}&run_id=${encodeURIComponent(runId)}`, 'GET');
        const tasks = Array.isArray(data?.tasks) ? data.tasks : [];
        if (!tasks.length) {
            setVmSelfDebugTimelineText(txt('vm_self_debug_timeline_empty', 'No self-debug timeline.'));
            setVmSelfDebugRunDetailText(txt('vm_self_debug_run_detail_empty', 'No run detail.'));
            return;
        }
        setVmSelfDebugTimelineText(buildSelfDebugTimelineText(tasks));
        const lines = tasks.map((t, i) => {
            const id = String(t?.id || '-');
            const kind = String(t?.run_kind || '-');
            const st = String(t?.status || '-');
            const ec = Number(t?.exit_code || 0);
            const cmd = String(t?.command || '');
            const msg = String(t?.message || '');
            const out = String(t?.output_preview || '');
            const cat = String(t?.failure_category || '');
            const sig = String(t?.failure_signature || '');
            const ssig = String(t?.strategy_signature || '');
            const trigger = String(t?.trigger_task_id || '');
            const keyLines = Array.isArray(t?.failure_key_lines) ? t.failure_key_lines.map((x) => String(x || '')).filter(Boolean) : [];
            return `#${i + 1} ${id} kind=${kind} [${st}] exit=${ec}\ncmd=${cmd}\nmsg=${msg}\ncat=${cat}\nsig=${sig}\nstrategy_sig=${ssig}\ntrigger=${trigger}\nkeys=${keyLines.join(' | ')}\nout=${out}`;
        });
        setVmSelfDebugRunDetailText(lines.join('\n\n'));
    } catch (e) {
        setVmSelfDebugTimelineText(fmt('status_vm_self_debug_timeline_failed', 'Load self-debug timeline failed: {error}', { error: String(e) }));
        setVmSelfDebugRunDetailText(fmt('status_vm_self_debug_run_detail_failed', 'Load self-debug run detail failed: {error}', { error: String(e) }));
    }
}

async function stopVmSelfDebugRun() {
    const name = selectedVmName();
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!runId) {
        setStatus(txt('status_vm_self_debug_run_id_required', 'Please enter self-debug run id first.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/self_debug/stop', 'POST', { name, run_id: runId });
        setStatus(txt('status_vm_self_debug_stop_ok', 'Self-debug stop requested.'), 'status-warn');
        await refreshVmExecQueue();
        await refreshVmSelfDebugRuns();
        await refreshVmSelfDebugHistory();
        await refreshVmSelfDebugStrategyStats();
        await refreshVmSelfDebugRunDetail();
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_stop_failed', 'Stop self-debug run failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function refreshVmSelfDebugContext() {
    const name = selectedVmName();
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    if (!name || !runId) {
        setVmSelfDebugContextText('');
        return;
    }
    try {
        const data = await vmApi(`/vm/self_debug/context?name=${encodeURIComponent(name)}&run_id=${encodeURIComponent(runId)}`, 'GET');
        setVmSelfDebugContextText(String(data?.context_text || ''));
    } catch (e) {
        setVmSelfDebugContextText(fmt('status_vm_self_debug_context_failed', 'Load self-debug context failed: {error}', { error: String(e) }));
    }
}

function quoteForEnvValue(raw) {
    return String(raw || '')
        .replace(/\\/g, '\\\\')
        .replace(/"/g, '\\"')
        .replace(/\$/g, '\\$')
        .replace(/`/g, '\\`')
        .replace(/\r?\n/g, ' | ');
}

function stripExistingContextPrefix(cmd) {
    return String(cmd || '').replace(/^KACF_SELF_DEBUG_CONTEXT="(?:\\.|[^"])*"\s+/, '');
}

async function applyVmSelfDebugContextToFixCmd() {
    const name = selectedVmName();
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!runId) {
        setStatus(txt('status_vm_self_debug_run_id_required', 'Please enter self-debug run id first.'), 'status-danger');
        return;
    }
    const fixInput = el('vm_self_debug_fix_cmd');
    if (!fixInput) return;
    try {
        const data = await vmApi(`/vm/self_debug/context?name=${encodeURIComponent(name)}&run_id=${encodeURIComponent(runId)}`, 'GET');
        const contextText = String(data?.context_text || '').trim();
        if (!contextText) {
            setStatus(txt('status_vm_self_debug_context_empty', 'Self-debug context is empty.'), 'status-danger');
            return;
        }
        const rawCurrent = String(fixInput.value || '').trim();
        const baseCmd = stripExistingContextPrefix(rawCurrent);
        if (!baseCmd) {
            setStatus(txt('status_vm_self_debug_fix_required', 'Please enter fix command for self-debug plan.'), 'status-danger');
            return;
        }
        const limited = quoteForEnvValue(contextText.slice(0, 1200));
        fixInput.value = `KACF_SELF_DEBUG_CONTEXT="${limited}" ${baseCmd}`;
        await refreshVmSelfDebugContext();
        setStatus(txt('status_vm_self_debug_context_applied', 'Failure context applied to fix command.'), 'status-warn');
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_context_apply_failed', 'Apply self-debug context failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function pauseVmSelfDebugRun() {
    const name = selectedVmName();
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!runId) {
        setStatus(txt('status_vm_self_debug_run_id_required', 'Please enter self-debug run id first.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/self_debug/pause', 'POST', { name, run_id: runId });
        setStatus(txt('status_vm_self_debug_pause_ok', 'Self-debug run paused.'), 'status-warn');
        await refreshVmExecQueue();
        await refreshVmSelfDebugRuns();
        await refreshVmSelfDebugHistory();
        await refreshVmSelfDebugRunDetail();
        await refreshVmSelfDebugContext();
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_pause_failed', 'Pause self-debug run failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function resumeVmSelfDebugRun() {
    const name = selectedVmName();
    const runId = String(el('vm_self_debug_run_id')?.value || '').trim();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!runId) {
        setStatus(txt('status_vm_self_debug_run_id_required', 'Please enter self-debug run id first.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/self_debug/resume', 'POST', { name, run_id: runId });
        setStatus(txt('status_vm_self_debug_resume_ok', 'Self-debug run resumed.'), 'status-warn');
        await refreshVmExecQueue();
        await refreshVmSelfDebugRuns();
        await refreshVmSelfDebugHistory();
        await refreshVmSelfDebugRunDetail();
        await refreshVmSelfDebugContext();
    } catch (e) {
        setStatus(fmt('status_vm_self_debug_resume_failed', 'Resume self-debug run failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function previewVmExecProfile() {
    const profile = String(el('vm_exec_profile')?.value || '').trim();
    const workdir = profileWorkdirInput();
    const testCmd = profileTestCmdInput();
    if (!profile) {
        setVmExecProfilePreviewText(txt('vm_exec_profile_preview_empty', 'No preview available.'));
        return;
    }
    try {
        const q = new URLSearchParams();
        q.set('profile', profile);
        if (workdir) q.set('workdir', workdir);
        if (testCmd) q.set('test_cmd', testCmd);
        const data = await vmApi(`/vm/exec/profile/preview?${q.toString()}`, 'GET');
        const tasks = Array.isArray(data?.tasks) ? data.tasks : [];
        if (!tasks.length) {
            setVmExecProfilePreviewText(txt('vm_exec_profile_preview_empty', 'No preview available.'));
            return;
        }
        const lines = tasks.map((cmd, i) => `#${i + 1} ${String(cmd || '')}`);
        setVmExecProfilePreviewText(lines.join('\n'));
    } catch (e) {
        setVmExecProfilePreviewText(fmt('status_vm_exec_profile_preview_failed', 'Profile preview failed: {error}', { error: String(e) }));
    }
}

async function enqueueVmExecProfile() {
    const name = selectedVmName();
    const profile = String(el('vm_exec_profile')?.value || '').trim();
    const workdir = profileWorkdirInput();
    const testCmd = profileTestCmdInput();
    const timeoutSec = asBoundedInt(el('vm_exec_timeout_sec')?.value, 1, 3600, 60);
    const waitReadySec = asBoundedInt(el('vm_exec_wait_ready_sec')?.value, 0, 600, 0);
    const priority = asBoundedSignedInt(el('vm_exec_priority')?.value, -100, 100, 0);
    const retryMax = asBoundedInt(el('vm_exec_retry_max')?.value, 0, 10, 0);
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!profile) {
        setStatus(txt('status_vm_exec_profile_required', 'Please select an exec profile first.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/exec/enqueue_profile', 'POST', {
            name,
            profile,
            workdir,
            test_cmd: testCmd,
            timeout_sec: timeoutSec,
            wait_ready_sec: waitReadySec,
            priority,
            retry_max: retryMax,
        });
        setStatus(fmt('status_vm_exec_profile_ok', 'Profile queued: {profile}', { profile: vmExecProfileLabel(profile) }), 'status-warn');
        await refreshVmExecQueue();
    } catch (e) {
        setStatus(fmt('status_vm_exec_queue_failed', 'VM exec queue operation failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function runNextVmExec() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    try {
        const res = await vmApi(`/vm/exec/queue/run_next?name=${encodeURIComponent(name)}`, 'POST');
        const lines = [];
        lines.push(`ok=${!!res.ok} exit=${Number(res.exit_code)}`);
        lines.push(`message=${res.message || ''}`);
        lines.push('--- stdout ---');
        lines.push(String(res.stdout || ''));
        lines.push('--- stderr ---');
        lines.push(String(res.stderr || ''));
        setVmExecOutput(lines.join('\n'));
        await refreshVmExecQueue();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_exec_queue_failed', 'VM exec queue operation failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function cancelVmQueueTask() {
    const name = selectedVmName();
    const taskId = (el('vm_exec_cancel_task_id')?.value || '').trim();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    if (!taskId) {
        setStatus(txt('status_vm_exec_task_id_required', 'Please enter queue task id first.'), 'status-danger');
        return;
    }
    try {
        await vmApi('/vm/exec/queue/cancel', 'POST', { name, task_id: taskId });
        setStatus(txt('status_vm_exec_queue_cancel_ok', 'Queue task cancel requested.'), 'status-warn');
        await refreshVmExecQueue();
    } catch (e) {
        setStatus(fmt('status_vm_exec_queue_failed', 'VM exec queue operation failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function checkVmReady() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    try {
        const data = await vmApi(`/vm/ready?name=${encodeURIComponent(name)}`, 'GET');
        if (data?.ok) {
            setStatus(txt('status_vm_ready_ok', 'VM SSH is ready.'), 'status-warn');
        } else {
            setStatus(fmt('status_vm_ready_fail', 'VM SSH is not ready: {error}', {
                error: String(data?.message || 'unknown'),
            }), 'status-danger');
        }
        await refreshVmStatus();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_ready_fail', 'VM SSH is not ready: {error}', {
            error: String(e),
        }), 'status-danger');
    }
}

async function bootstrapVm() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    const confirmText = fmt('confirm_vm_bootstrap', 'Bootstrap VM "{name}" now?', { name });
    if (!window.confirm(confirmText)) return;
    setVmExecOutput('');
    try {
        const res = await vmApi('/vm/bootstrap', 'POST', { name, profile: 'dev-basic' });
        const lines = [];
        lines.push(`ok=${!!res.ok} exit=${Number(res.exit_code)}`);
        lines.push(`message=${res.message || ''}`);
        lines.push('--- stdout ---');
        lines.push(String(res.stdout || ''));
        lines.push('--- stderr ---');
        lines.push(String(res.stderr || ''));
        setVmExecOutput(lines.join('\n'));
        if (res.ok) {
            setStatus(txt('status_vm_bootstrap_ok', 'VM bootstrap finished.'), 'status-warn');
        } else {
            setStatus(txt('status_vm_bootstrap_nonzero', 'VM bootstrap finished with non-zero exit.'), 'status-danger');
        }
        await refreshVmStatus();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_bootstrap_failed', 'VM bootstrap failed: {error}', { error: String(e) }), 'status-danger');
        setVmExecOutput(String(e));
    }
}

async function cancelVmExec() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    try {
        const res = await vmApi('/vm/exec/cancel', 'POST', { name });
        setStatus(res?.message || txt('status_vm_exec_cancel_ok', 'Cancel request sent.'), 'status-warn');
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_exec_cancel_failed', 'Cancel VM exec failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function refreshVmLogs() {
    const name = selectedVmName();
    if (!name) {
        setVmLogsText(txt('vm_log_empty', 'No VM selected.'));
        return;
    }
    try {
        const data = await vmApi(`/vm/logs?name=${encodeURIComponent(name)}&tail=12000`, 'GET');
        const text = String(data?.text || '').trim();
        if (!text) {
            setVmLogsText(txt('vm_log_empty', 'No logs yet.'));
            return;
        }
        setVmLogsText(text);
    } catch (e) {
        setVmLogsText(fmt('status_vm_log_fetch_failed', 'Fetch VM logs failed: {error}', { error: String(e) }));
    }
}

async function refreshVmExecQueue() {
    const name = selectedVmName();
    if (!name) {
        setVmExecQueueText(txt('vm_exec_queue_empty', 'No queued task.'));
        setVmExecQueueStatsText('');
        return;
    }
    try {
        const data = await vmApi(`/vm/exec/queue?name=${encodeURIComponent(name)}`, 'GET');
        const list = Array.isArray(data?.items) ? data.items : [];
        if (!list.length) {
            setVmExecQueueText(txt('vm_exec_queue_empty', 'No queued task.'));
        } else {
            const lines = list.map((x, i) => {
                const id = String(x?.id || '-');
                const st = String(x?.status || '-');
                const ec = Number(x?.exit_code || 0);
                const p = Number(x?.priority || 0);
                const rc = Number(x?.retry_count || 0);
                const rm = Number(x?.retry_max || 0);
                const nextRun = Number(x?.next_run_after_unix || 0);
                const runId = String(x?.run_id || '');
                const cmd = String(x?.command || '').slice(0, 120);
                const runTag = runId ? ` run=${runId}` : '';
                return `#${i + 1} ${id} [${st}] p=${p} retry=${rc}/${rm} next=${nextRun} exit=${ec}${runTag} cmd=${cmd}`;
            });
            setVmExecQueueText(lines.join('\n'));
        }
        await refreshVmExecQueueStats();
    } catch (e) {
        setVmExecQueueText(fmt('status_vm_exec_queue_failed', 'VM exec queue operation failed: {error}', { error: String(e) }));
        setVmExecQueueStatsText('');
    }
}

async function refreshVmExecQueueStats() {
    const name = selectedVmName();
    if (!name) {
        setVmExecQueueStatsText('');
        return;
    }
    try {
        const s = await vmApi(`/vm/exec/queue/stats?name=${encodeURIComponent(name)}`, 'GET');
        const line = fmt('vm_exec_queue_stats_line', 'Queue total={total} pending={pending} running={running} done={done} failed={failed} canceled={canceled} ok={ok_rate}% avg={avg_sec}s', {
            total: Number(s?.total || 0),
            pending: Number(s?.pending || 0),
            running: Number(s?.running || 0),
            done: Number(s?.done || 0),
            failed: Number(s?.failed || 0),
            canceled: Number(s?.canceled || 0),
            ok_rate: Number(s?.done_success_rate || 0).toFixed(1),
            avg_sec: Number(s?.avg_duration_sec || 0).toFixed(1),
        });
        setVmExecQueueStatsText(line);
    } catch (_e) {
        setVmExecQueueStatsText('');
    }
}

async function provisionVm() {
    const name = (el('vm_name')?.value || '').trim();
    if (!name) {
        setStatus(txt('status_vm_name_required', 'VM name is required'), 'status-danger');
        return;
    }
    try {
        const body = {
            name,
            backend: (el('vm_backend')?.value || '').trim(),
            os_image: (el('vm_os_image')?.value || '').trim(),
            ssh_user: (el('vm_ssh_user')?.value || '').trim(),
            cpu: asPositiveInt(el('vm_cpu')?.value, 2),
            memory_mb: asPositiveInt(el('vm_memory_mb')?.value, 4096),
            disk_gb: asPositiveInt(el('vm_disk_gb')?.value, 40),
        };
        const res = await vmApi('/vm/provision', 'POST', body);
        setStatus(txt('status_vm_provision_ok', 'VM provisioned'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'provision ok' }));
        await refreshVmStatus();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_provision_failed', 'VM provision failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function startVm() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    try {
        const res = await vmApi('/vm/start', 'POST', { name });
        setStatus(txt('status_vm_start_ok', 'VM start request submitted'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'start ok' }));
        await refreshVmStatus();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_start_failed', 'VM start failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function stopVm() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    try {
        const res = await vmApi('/vm/stop', 'POST', { name });
        setStatus(txt('status_vm_stop_ok', 'VM stop request submitted'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'stop ok' }));
        await refreshVmStatus();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_stop_failed', 'VM stop failed: {error}', { error: String(e) }), 'status-danger');
    }
}

async function deleteVm() {
    const name = selectedVmName();
    if (!name) {
        setStatus(txt('status_vm_target_required', 'Please select a VM first'), 'status-danger');
        return;
    }
    const confirmText = fmt('confirm_vm_delete', 'Delete VM "{name}"?', { name });
    if (!window.confirm(confirmText)) return;
    try {
        const purgeDisk = !!el('vm_purge_disk')?.checked;
        const res = await vmApi('/vm/delete', 'POST', { name, purge_disk: purgeDisk });
        setStatus(txt('status_vm_delete_ok', 'VM deleted'), 'status-warn');
        appendLog(fmt('log_vm_action', '[VM] {message}', { message: res.message || 'delete ok' }));
        await refreshVmStatus();
        await refreshVmLogs();
    } catch (e) {
        setStatus(fmt('status_vm_delete_failed', 'VM delete failed: {error}', { error: String(e) }), 'status-danger');
    }
}

function bindVmEvents() {
    if (vmBound) return;
    vmBound = true;
    el('vm_refresh_btn')?.addEventListener('click', refreshVmStatus);
    el('vm_ready_btn')?.addEventListener('click', checkVmReady);
    el('vm_bootstrap_btn')?.addEventListener('click', bootstrapVm);
    el('vm_provision_btn')?.addEventListener('click', provisionVm);
    el('vm_start_btn')?.addEventListener('click', startVm);
    el('vm_stop_btn')?.addEventListener('click', stopVm);
    el('vm_delete_btn')?.addEventListener('click', deleteVm);
    el('vm_refresh_logs_btn')?.addEventListener('click', refreshVmLogs);
    el('vm_snapshot_refresh_btn')?.addEventListener('click', refreshVmSnapshots);
    el('vm_snapshot_create_btn')?.addEventListener('click', createVmSnapshot);
    el('vm_snapshot_apply_btn')?.addEventListener('click', applyVmSnapshot);
    el('vm_snapshot_delete_btn')?.addEventListener('click', deleteVmSnapshot);
    el('vm_clone_btn')?.addEventListener('click', cloneVmFromSnapshot);
    el('vm_exec_btn')?.addEventListener('click', execInVm);
    el('vm_exec_cancel_btn')?.addEventListener('click', cancelVmExec);
    el('vm_exec_enqueue_btn')?.addEventListener('click', enqueueVmExec);
    el('vm_exec_batch_enqueue_btn')?.addEventListener('click', enqueueVmExecBatch);
    el('vm_exec_profile_enqueue_btn')?.addEventListener('click', enqueueVmExecProfile);
    el('vm_exec_profile_preview_btn')?.addEventListener('click', previewVmExecProfile);
    el('vm_exec_custom_profile_load_btn')?.addEventListener('click', loadSelectedProfileToEditor);
    el('vm_exec_custom_profile_save_btn')?.addEventListener('click', saveCustomProfile);
    el('vm_exec_custom_profile_delete_btn')?.addEventListener('click', deleteCustomProfile);
    el('vm_self_debug_preview_btn')?.addEventListener('click', previewVmSelfDebugPlan);
    el('vm_self_debug_start_btn')?.addEventListener('click', startVmSelfDebugPlan);
    el('vm_self_debug_runs_refresh_btn')?.addEventListener('click', refreshVmSelfDebugRuns);
    el('vm_self_debug_history_refresh_btn')?.addEventListener('click', refreshVmSelfDebugHistory);
    el('vm_self_debug_history_detail_btn')?.addEventListener('click', loadVmSelfDebugHistoryDetail);
    el('vm_self_debug_history_archive_btn')?.addEventListener('click', archiveVmSelfDebugCompletedRuns);
    el('vm_self_debug_history_clear_btn')?.addEventListener('click', clearVmSelfDebugHistory);
    el('vm_self_debug_stats_btn')?.addEventListener('click', refreshVmSelfDebugStrategyStats);
    el('vm_self_debug_detail_btn')?.addEventListener('click', refreshVmSelfDebugRunDetail);
    el('vm_self_debug_context_btn')?.addEventListener('click', refreshVmSelfDebugContext);
    el('vm_self_debug_apply_context_btn')?.addEventListener('click', applyVmSelfDebugContextToFixCmd);
    el('vm_self_debug_pause_btn')?.addEventListener('click', pauseVmSelfDebugRun);
    el('vm_self_debug_resume_btn')?.addEventListener('click', resumeVmSelfDebugRun);
    el('vm_self_debug_stop_btn')?.addEventListener('click', stopVmSelfDebugRun);
    el('vm_self_debug_rules_load_btn')?.addEventListener('click', loadVmSelfDebugStrategyRules);
    el('vm_self_debug_rules_save_btn')?.addEventListener('click', saveVmSelfDebugStrategyRules);
    el('vm_exec_run_next_btn')?.addEventListener('click', runNextVmExec);
    el('vm_exec_queue_refresh_btn')?.addEventListener('click', refreshVmExecQueue);
    el('vm_exec_queue_cancel_btn')?.addEventListener('click', cancelVmQueueTask);
    el('vm_target_select')?.addEventListener('change', async () => {
        setVmReadOnlyByContext();
        await refreshVmSnapshots();
        await refreshVmExecQueue();
        await refreshVmLogs();
        await refreshVmSelfDebugRuns();
        await refreshVmSelfDebugHistory();
        await refreshVmSelfDebugStrategyStats();
        await refreshVmSelfDebugRunDetail();
        await refreshVmSelfDebugContext();
    });
    el('vm_exec_profile')?.addEventListener('change', previewVmExecProfile);
    el('vm_exec_profile_workdir')?.addEventListener('input', previewVmExecProfile);
    el('vm_exec_profile_test_cmd')?.addEventListener('input', previewVmExecProfile);
    el('vm_self_debug_run_id')?.addEventListener('input', async () => {
        await refreshVmSelfDebugRunDetail();
        await refreshVmSelfDebugContext();
    });
    setInterval(setVmReadOnlyByContext, 1000);
}

async function init(opts) {
    const options = opts || {};
    vmGuestMode = !!options.guestMode;
    const card = el('vm_card');
    if (!card) return;
    if (vmGuestMode) {
        card.style.display = 'none';
        return;
    }
    bindVmEvents();
    await refreshVmStatus();
    if (!vmRefreshTimer) {
        vmRefreshTimer = setInterval(refreshVmStatus, 15000);
    }
    if (!vmLogRefreshTimer) {
        vmLogRefreshTimer = setInterval(refreshVmLogs, 8000);
    }
    if (!vmQueueRefreshTimer) {
        vmQueueRefreshTimer = setInterval(refreshVmExecQueue, 4000);
    }
    await refreshVmExecProfiles();
    await refreshVmSnapshots();
    await refreshVmExecQueue();
    await refreshVmSelfDebugRuns();
    await refreshVmSelfDebugHistory();
    await refreshVmSelfDebugStrategyStats();
    await loadVmSelfDebugStrategyRules();
    await refreshVmSelfDebugRunDetail();
    await refreshVmSelfDebugContext();
}

window.KACF = window.KACF || {};
window.KACF.vm = {
    init,
    refreshVmStatus,
    refreshVmLogs,
};
})();

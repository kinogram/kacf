function markBackendFailure(source, immediate) {
    if (immediate) {
        backendFailureCount = BACKEND_FAILURE_THRESHOLD;
    } else {
        backendFailureCount += 1;
    }
    if (backendFailureCount < BACKEND_FAILURE_THRESHOLD) return;
    if (backendOfflineNotified) return;
    backendOfflineNotified = true;
    setBackendOfflineState(true);
    const state = currentRunState();
    const runningLike = isRunStateActive(state);
    if (runningLike) {
        syncStoppedStateIfNeeded(fmt('sync_reason_backend_disconnected', '', { source }), true);
        applyInterruptedTerminalState({
            runTextKey: 'run_interrupted',
            statusKey: 'status_backend_disconnected',
            statusClass: 'status-danger',
        });
    } else {
        setStatus(txt('status_backend_offline', ''), 'status-danger');
    }
    alert(txt(runningLike ? 'alert_backend_disconnected' : 'alert_backend_offline', ''));
}

async function checkBackendHealth() {
    try {
        const resp = await fetch('/health', { cache: 'no-store' });
        if (!resp.ok) throw new Error(`health ${resp.status}`);
        markBackendAlive();
    } catch (_e) {
        markBackendFailure('health', true);
    }
}

function syncStoppedStateIfNeeded(reason, suppressStoppedAlert) {
    const state = currentRunState();
    if (!isRunStateActive(state)) return;
    finishRunSessionUi();
    setRunState('idle', txt('run_idle', ''));
    if (!suppressStoppedAlert) {
        showAlertOnce(txt('alert_task_stopped', ''));
    }
    if (reason) {
        appendLog(fmt('log_runtime_sync', '', { reason }));
    }
}

function isInterruptedMessage(msg) {
    const text = (msg || '').toLowerCase();
    const raw = txt(
        'interrupt_keywords',
        'stopped by user|interrupted|stop requested|cancelled|canceled|aborted|terminated|中断|停止|已停止|detenido|interrumpido|arrete|annule'
    );
    const keywords = raw
        .split('|')
        .map(x => x.trim().toLowerCase())
        .filter(Boolean);
    return keywords.some(k => text.includes(k));
}

function getFormData() {
    return {
        api_key: getApiKeyInputValue(),
        base_url: document.getElementById('base_url').value.trim(),
        model: document.getElementById('model').value.trim(),
        language: normalizeLanguageCode(sharedConfig.language || currentLanguage),
        auto_revert_profile: document.getElementById('auto_revert_profile').value.trim(),
        precheck_cmd: document.getElementById('precheck_cmd').value.trim(),
        release_gate_threshold: document.getElementById('release_gate_threshold').value.trim(),
        unattended_mode: !!document.getElementById('unattended_mode')?.checked,
        workspace: document.getElementById('workspace').value.trim(),
        goal: document.getElementById('goal').value.trim(),
        eval_cmd: document.getElementById('eval_cmd').value.trim(),
        success_regex: document.getElementById('success_regex').value.trim(),
        remote: document.getElementById('remote').value.trim(),
        remote_url: document.getElementById('remote_url').value.trim(),
        branch: document.getElementById('branch').value.trim(),
    };
}

function applyFormData(d) {
    if (!d) return;
    const fields = ['api_key', 'base_url', 'model', 'auto_revert_profile', 'precheck_cmd', 'release_gate_threshold', 'workspace', 'goal', 'eval_cmd', 'success_regex', 'remote', 'remote_url', 'branch'];
    fields.forEach(k => {
        if (typeof d[k] === 'string' && document.getElementById(k)) {
            document.getElementById(k).value = d[k];
        }
    });
    if (typeof d.unattended_mode === 'boolean') {
        const el = document.getElementById('unattended_mode');
        if (el) el.checked = d.unattended_mode;
    }
    if (typeof d.language === 'string' && d.language) {
        const code = normalizeLanguageCode(d.language);
        if (code) sharedConfig.language = code;
    }
    renderUnattendedState();
}

function applyStaticCopyToDom() {
    document.title = txt('app_title', document.title);
    const h1 = document.querySelector('.hero h1');
    if (h1) h1.textContent = txt('app_header', h1.textContent);
    const staticMap = [
        ['hero_sub', 'hero_sub'],
        ['backend_offline_banner', 'banner_backend_offline'],
        ['section_projects_title', 'section_projects_title'],
        ['section_projects_sub', 'section_projects_sub'],
        ['section_config_title', 'section_config_title'],
        ['section_config_sub', 'section_config_sub'],
        ['label_project_name', 'label_project_name'],
        ['label_unattended_mode', 'label_unattended_mode'],
        ['label_global_config', 'label_global_config'],
        ['label_auto_revert_profile', 'label_auto_revert_profile'],
        ['label_precheck_cmd', 'label_precheck_cmd'],
        ['label_release_gate_threshold', 'label_release_gate_threshold'],
        ['label_history_max_messages', 'label_history_max_messages'],
        ['label_history_max_chars', 'label_history_max_chars'],
        ['label_goal', 'label_goal'],
        ['label_eval_cmd', 'label_eval_cmd'],
        ['label_success_regex', 'label_success_regex'],
        ['section_git_title', 'section_git_title'],
        ['label_remote', 'label_remote'],
        ['label_remote_url', 'label_remote_url'],
        ['label_branch', 'label_branch'],
        ['hint_run_new_round', 'hint_run_new_round'],
        ['section_status_title', 'section_status_title'],
        ['unattended_state_text', 'unattended_state_default'],
        ['section_clarify_title', 'section_clarify_title'],
        ['section_log_title', 'section_log_title'],
        ['hint_log_exports', 'hint_log_exports'],
        ['section_diff_title', 'section_diff_title'],
        ['section_global_title', 'section_global_title'],
        ['label_api_key', 'label_api_key'],
        ['label_model', 'label_model'],
        ['label_base_url', 'label_base_url'],
        ['label_language', 'label_language'],
        ['label_global_auto_resume_attempts', 'label_global_auto_resume_attempts'],
        ['label_global_stop_after_minutes', 'label_global_stop_after_minutes'],
        ['label_global_log_max_chars', 'label_global_log_max_chars'],
        ['label_global_diff_max_chars', 'label_global_diff_max_chars'],
    ];
    staticMap.forEach(([id, key]) => {
        const el = document.getElementById(id);
        if (el) el.textContent = txt(key, '');
    });
    const iconMap = [
        ['start_btn', 'btn_start', '&#9654;'],
        ['resume_btn', 'btn_resume', '&#8635;'],
        ['stop_btn', 'btn_stop', '&#9632;'],
        ['go_running_project_btn', 'btn_go_running', '&#9679;'],
        ['revert_btn', 'btn_revert', '&#8630;'],
        ['push_btn', 'btn_push', '&#8679;'],
        ['open_global_config_btn', 'btn_open_global', '&#9881;'],
        ['save_global_config_btn', 'btn_save_global', '&#10003;'],
        ['close_global_config_btn', 'btn_close', '&#10005;'],
        ['project_new_btn', 'btn_project_new', '&#43;'],
        ['project_save_btn', 'btn_project_save', '&#10003;'],
        ['project_load_btn', 'btn_project_load', '&#10548;'],
        ['project_delete_btn', 'btn_project_delete', '&#128465;'],
        ['clarify_submit_btn', 'btn_clarify_submit', '&#10148;'],
        ['clear_log_btn', 'btn_clear_log', '&#128465;'],
        ['export_log_btn', 'btn_export_log', '&#10515;'],
        ['export_snapshot_btn', 'btn_export_snapshot', '&#10515;'],
        ['export_report_btn', 'btn_export_report', '&#10515;'],
    ];
    iconMap.forEach(([id, key, icon]) => {
        const el = document.getElementById(id);
        if (!el) return;
        const label = txt(key, '');
        el.classList.add('btn-icon-only');
        el.innerHTML = `<span class="btn-icon" aria-hidden="true">${icon}</span>`;
        el.title = label;
        el.setAttribute('aria-label', label);
    });
    const logBtnTips = [
        ['export_log_btn', 'tip_export_log'],
        ['export_snapshot_btn', 'tip_export_snapshot'],
        ['export_report_btn', 'tip_export_report'],
    ];
    logBtnTips.forEach(([id, key]) => {
        const el = document.getElementById(id);
        if (el) el.title = txt(key, '');
    });
    const placeholderMap = [
        ['project_name', 'ph_project_name'],
        ['precheck_cmd', 'ph_precheck_cmd'],
        ['release_gate_threshold', 'ph_release_gate_threshold'],
        ['global_auto_resume_attempts', 'ph_global_auto_resume_attempts'],
        ['global_stop_after_minutes', 'ph_global_stop_after_minutes'],
        ['global_history_max_messages', 'ph_history_max_messages'],
        ['global_history_max_chars', 'ph_history_max_chars'],
        ['global_log_max_chars', 'ph_global_log_max_chars'],
        ['global_diff_max_chars', 'ph_global_diff_max_chars'],
        ['goal', 'ph_goal'],
        ['success_regex', 'ph_success_regex'],
        ['api_key', 'ph_api_key'],
        ['project_search', 'project_search_placeholder']
    ];
    placeholderMap.forEach(([id, key]) => {
        const el = document.getElementById(id);
        if (el) el.placeholder = txt(key, '');
    });
    const search = document.getElementById('project_search');
    if (search) search.placeholder = txt('project_search_placeholder', '');
    const summary = document.getElementById('project_summary_text');
    if (summary) {
        summary.textContent = fmt('project_summary_total', '', { total: loadProjects().length });
    }
    setProjectDraftState();
    renderProjectAccordion();
    applySidebarLayout();
    renderUnattendedState();
}

function openGlobalConfigModal() {
    closeSidebarOnNarrow();
    const modal = document.getElementById('global_config_modal');
    if (!modal) return;
    modal.style.display = 'flex';
    document.body.classList.add('modal-open');
}

function closeGlobalConfigModal() {
    const modal = document.getElementById('global_config_modal');
    if (!modal) return;
    modal.style.display = 'none';
    document.body.classList.remove('modal-open');
}

async function saveGlobalConfig() {
    updateGlobalOptionsFromInputs();
    maybeWarnLargeCharLimit(txt('label_global_config', ''));
    if (logMaxCharsSetting() > LARGE_CHAR_LIMIT_WARNING_THRESHOLD || diffMaxCharsSetting() > LARGE_CHAR_LIMIT_WARNING_THRESHOLD) {
        alert(fmt('warn_large_char_limit_alert', '', {
            threshold: LARGE_CHAR_LIMIT_WARNING_THRESHOLD,
            log_limit: logMaxCharsSetting(),
            diff_limit: diffMaxCharsSetting(),
        }));
    }
    const wantedLanguage = normalizeLanguageCode(document.getElementById('language_select')?.value || currentLanguage);
    if (wantedLanguage && wantedLanguage !== currentLanguage) {
        const switched = await switchLanguage(wantedLanguage, false);
        if (!switched) {
            setStatus(fmt('status_language_switch_failed', '', { code: wantedLanguage }), 'status-danger');
            return;
        }
    }
    const ok = await persistSharedConfigNow();
    if (!ok) {
        setStatus(txt('status_global_save_failed', ''), 'status-danger');
        return;
    }
    closeGlobalConfigModal();
    setStatus(txt('status_global_saved', ''), 'status-warn');
    renderCurrentLogView();
    renderUiForViewBucket();
}

function setAutoSaveState(text) {
    document.getElementById('auto_save_state').textContent = text;
}

function renderUnattendedState() {
    const el = document.getElementById('unattended_state_text');
    if (!el) return;
    const configuredUnattendedMode = !!document.getElementById('unattended_mode')?.checked;
    const unattendedMode = runSessionActive ? activeRunUnattendedMode : configuredUnattendedMode;
    const mins = stopAfterMinutesSetting();
    let stopText = txt('unattended_stop_disabled', '');
    if (mins > 0) {
        if (!runSessionActive) {
            stopText = fmt('unattended_stop_left_minutes', '', { minutes: mins });
        } else if (manualRunStopExpired) {
            stopText = txt('unattended_stop_expired', '');
        } else {
            const remaining = manualRunStopDeadlineMs > 0
                ? Math.max(0, Math.ceil((manualRunStopDeadlineMs - Date.now()) / 60000))
                : mins;
            stopText = fmt('unattended_stop_left_minutes', '', { minutes: remaining });
        }
    }
    const resumeLeft = runSessionActive
        ? Math.max(0, unattendedAutoResumeRemaining)
        : Math.max(0, autoResumeAttemptsSetting());
    el.textContent = fmt('unattended_state_line', '', {
        mode: unattendedMode ? txt('unattended_mode_on', '') : txt('unattended_mode_off', ''),
        resume_left: String(resumeLeft),
        stop: stopText,
    });
}

function clearUnattendedAutoResumeTimer() {
    if (!unattendedAutoResumeTimer) return;
    clearTimeout(unattendedAutoResumeTimer);
    unattendedAutoResumeTimer = null;
}

function clearManualRunStopTimer() {
    if (!manualRunStopTimer) return;
    clearTimeout(manualRunStopTimer);
    manualRunStopTimer = null;
}

function stopAfterMinutesSetting() {
    return parseBoundedInt(globalOptions.stop_after_minutes, 0, 24 * 60, 0);
}

function armManualRunStopTimer() {
    clearManualRunStopTimer();
    manualRunStopExpired = false;
    const mins = stopAfterMinutesSetting();
    if (mins <= 0) {
        manualRunStopDeadlineMs = 0;
        renderUnattendedState();
        return;
    }
    manualRunStopDeadlineMs = Date.now() + mins * 60 * 1000;
    renderUnattendedState();
    manualRunStopTimer = setTimeout(() => {
        manualRunStopExpired = true;
        appendLog(fmt('log_stop_timer_expired', '', { minutes: mins }));
        setStatus(fmt('status_stop_timer_wait_round', '', { minutes: mins }), 'status-warn');
        renderUnattendedState();
    }, mins * 60 * 1000);
}

async function triggerStopByTimeout() {
    if (!runSessionActive || stopRequested) return;
    manualRunStopExpired = false;
    appendLog(txt('log_stop_timer_trigger_stop', ''));
    renderUnattendedState();
    await stopSession();
}

function maybeStopOnRoundBoundary(line) {
    if (!manualRunStopExpired) return;
    if (!line.startsWith('[Loop] Iteration')) return;
    triggerStopByTimeout();
}

function autoResumeAttemptsSetting() {
    return parseBoundedInt(globalOptions.auto_resume_attempts, 0, 20, 0);
}

function scheduleUnattendedAutoResume() {
    if (!activeRunUnattendedMode || unattendedAutoResumeRemaining <= 0) return false;
    clearUnattendedAutoResumeTimer();
    const nextTry = unattendedAutoResumeRemaining;
    unattendedAutoResumeRemaining -= 1;
    renderUnattendedState();
    appendLog(fmt('log_unattended_auto_resume_scheduled', '', {
        try: nextTry,
        left: unattendedAutoResumeRemaining,
    }));
    unattendedAutoResumeTimer = setTimeout(() => {
        resumeSession({ auto: true });
    }, 1200);
    return true;
}

function resetStopTimerState() {
    clearManualRunStopTimer();
    manualRunStopDeadlineMs = 0;
    manualRunStopExpired = false;
}

function resetUnattendedRunState() {
    activeRunUnattendedMode = false;
    unattendedAutoResumeRemaining = 0;
    clearUnattendedAutoResumeTimer();
    resetStopTimerState();
    renderUnattendedState();
}

function resetRunSessionUiState() {
    activeRunLogBucket = '';
    activeRunProjectLabel = '';
    runSessionActive = false;
    setRunningProjectIndicator(txt('running_project_none', ''));
    setProjectControlsDisabled(false);
    applyReadOnlyMode();
}

function showAlertOnce(message) {
    if (completionAlertShown) return false;
    completionAlertShown = true;
    alert(message);
    return true;
}

function setRunStateAndStatus(runState, runTextKey, statusKey, statusClass, message) {
    setRunState(runState, txt(runTextKey, ''));
    if (typeof message === 'string') {
        setStatus(fmt(statusKey, '', { message }), statusClass);
    } else {
        setStatus(txt(statusKey, ''), statusClass);
    }
}

function finishRunSessionUi() {
    clearStopAckTimer();
    stopRequested = false;
    resetRunSessionUiState();
    setRunActionButtons(false);
}

function markRunSessionStarted(label, resetFlags) {
    setRunningProjectIndicator(label);
    setProjectControlsDisabled(true);
    runSessionActive = true;
    applyReadOnlyMode();
    setRunActionButtons(true);
    document.getElementById('revert_btn').disabled = false;
    if (resetFlags) {
        stopRequested = false;
        completionAlertShown = false;
    }
}

const DONE_OUTCOME_CONFIG = {
    success: {
        runState: 'success',
        runTextKey: 'run_success',
        statusKey: 'status_done_success',
        statusClass: 'status-ok',
        alertKey: 'alert_done_success',
        resetUnattended: true,
        allowAutoResume: false,
    },
    interrupted: {
        runState: 'interrupted',
        runTextKey: 'run_interrupted',
        statusKey: 'status_done_interrupted',
        statusClass: 'status-warn',
        alertKey: 'alert_done_interrupted',
        resetUnattended: true,
        allowAutoResume: false,
    },
    failed: {
        runState: 'failed',
        runTextKey: 'run_failed',
        statusKey: 'status_done_failed',
        statusClass: 'status-danger',
        alertKey: 'alert_done_failed',
        resetUnattended: false,
        allowAutoResume: true,
    },
};

function applyDoneOutcome(outcome, message) {
    const cfg = DONE_OUTCOME_CONFIG[outcome] || DONE_OUTCOME_CONFIG.failed;
    setRunStateAndStatus(cfg.runState, cfg.runTextKey, cfg.statusKey, cfg.statusClass, message);
    if (cfg.resetUnattended) {
        resetUnattendedRunState();
    }
    let autoResumed = false;
    if (cfg.allowAutoResume) {
        autoResumed = scheduleUnattendedAutoResume();
    }
    if (!autoResumed) {
        showAlertOnce(fmt(cfg.alertKey, '', { message }));
    }
}

function applyInterruptedTerminalState(options) {
    const cfg = options || {};
    setRunStateAndStatus(
        'interrupted',
        cfg.runTextKey || 'run_interrupted',
        cfg.statusKey || 'status_done_interrupted',
        cfg.statusClass || 'status-warn',
        cfg.message
    );
    if (cfg.alertKey) {
        showAlertOnce(
            typeof cfg.message === 'string'
                ? fmt(cfg.alertKey, '', { message: cfg.message })
                : txt(cfg.alertKey, '')
        );
    }
}

const STOP_ACCEPTED_STATUS_CONFIG = {
    accepted: { key: 'status_stop_waiting_ack', className: 'status-warn' },
    channel_unavailable: { key: 'status_stop_backend_done', className: 'status-warn' },
};

function applyStopAcceptedStatus(ack) {
    const cfg = ack.includes('channel unavailable')
        ? STOP_ACCEPTED_STATUS_CONFIG.channel_unavailable
        : STOP_ACCEPTED_STATUS_CONFIG.accepted;
    setRunState('stopping', txt('run_stopping', ''));
    setStatus(txt(cfg.key, ''), cfg.className);
    setRunActionButtons(false);
}

window.KACF = window.KACF || {};
window.KACF.runtimeState = {
    markBackendFailure,
    checkBackendHealth,
    syncStoppedStateIfNeeded,
    isInterruptedMessage,
    getFormData,
    applyFormData,
    applyStaticCopyToDom,
    openGlobalConfigModal,
    closeGlobalConfigModal,
    saveGlobalConfig,
    setAutoSaveState,
    renderUnattendedState,
    clearUnattendedAutoResumeTimer,
    clearManualRunStopTimer,
    stopAfterMinutesSetting,
    armManualRunStopTimer,
    triggerStopByTimeout,
    maybeStopOnRoundBoundary,
    autoResumeAttemptsSetting,
    scheduleUnattendedAutoResume,
    resetStopTimerState,
    resetUnattendedRunState,
    resetRunSessionUiState,
    showAlertOnce,
    setRunStateAndStatus,
    finishRunSessionUi,
    markRunSessionStarted,
    applyDoneOutcome,
    applyInterruptedTerminalState,
    applyStopAcceptedStatus,
};

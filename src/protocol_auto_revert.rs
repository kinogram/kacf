use std::cell::RefCell;

thread_local! {
    static CURRENT_AUTO_REVERT_PROFILE: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(crate) fn set_current_auto_revert_profile(profile: &str) {
    CURRENT_AUTO_REVERT_PROFILE.with(|v| {
        *v.borrow_mut() = Some(profile.to_lowercase());
    });
}

pub(crate) fn auto_revert_on_repeat() -> bool {
    bool_env("AUTOCODING_AUTO_REVERT_ON_REPEAT", false)
}

pub(crate) fn auto_revert_repeat_count() -> u32 {
    read_u32_env("AUTOCODING_AUTO_REVERT_REPEAT_COUNT", 2, 20).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 4,
            "aggressive" => 2,
            _ => 3,
        }
    })
}

pub(crate) fn auto_revert_min_severity() -> u8 {
    read_u8_env("AUTOCODING_AUTO_REVERT_MIN_SEVERITY", 0, 3).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 3,
            "aggressive" => 1,
            _ => 2,
        }
    })
}

pub(crate) fn auto_revert_on_worse() -> bool {
    bool_env("AUTOCODING_AUTO_REVERT_ON_WORSE", true)
}

pub(crate) fn auto_revert_min_worse_delta() -> u8 {
    read_u8_env("AUTOCODING_AUTO_REVERT_MIN_WORSE_DELTA", 0, 3).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 2,
            "aggressive" => 0,
            _ => 1,
        }
    })
}

pub(crate) fn auto_revert_cooldown_iters() -> u32 {
    read_u32_env("AUTOCODING_AUTO_REVERT_COOLDOWN_ITERS", 0, 20).unwrap_or_else(|| {
        match auto_revert_profile().as_str() {
            "conservative" => 3,
            "aggressive" => 1,
            _ => 2,
        }
    })
}

pub(crate) fn auto_revert_signature_allowed(signature: &str) -> bool {
    let sig = signature.to_lowercase();
    let deny = read_keyword_list("AUTOCODING_AUTO_REVERT_SIGNATURE_DENY");
    if !deny.is_empty() && deny.iter().any(|k| sig.contains(k)) {
        return false;
    }
    let allow = read_keyword_list("AUTOCODING_AUTO_REVERT_SIGNATURE_ALLOW");
    if allow.is_empty() {
        return true;
    }
    allow.iter().any(|k| sig.contains(k))
}

fn auto_revert_profile() -> String {
    if let Some(cfg_profile) = CURRENT_AUTO_REVERT_PROFILE.with(|v| v.borrow().clone()) {
        return cfg_profile;
    }
    std::env::var("AUTOCODING_AUTO_REVERT_PROFILE")
        .ok()
        .map(|v| v.to_lowercase())
        .filter(|v| matches!(v.as_str(), "conservative" | "balanced" | "aggressive"))
        .unwrap_or_else(|| "balanced".to_string())
}

fn read_u32_env(key: &str, min: u32, max: u32) -> Option<u32> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v >= min && *v <= max)
}

fn read_u8_env(key: &str, min: u8, max: u8) -> Option<u8> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v >= min && *v <= max)
}

fn bool_env(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn read_keyword_list(key: &str) -> Vec<String> {
    std::env::var(key)
        .ok()
        .map(|v| {
            v.split(',')
                .map(|x| x.trim().to_lowercase())
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

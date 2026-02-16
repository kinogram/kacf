use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const LANGUAGES_DIR: &str = "static/languages";
const LANGUAGE_INPUTER_VERSION: &str = "00003";
const LANGUAGE_PACK_PREFIX: &str = "KACF_";

#[derive(Debug, Clone)]
struct LoadedLanguagePack {
    code: String,
    path: PathBuf,
    content: String,
    keys: BTreeSet<String>,
}

pub(crate) fn ensure_language_packs_checked() -> Result<(), String> {
    let _ = load_language_packs_checked()?;
    Ok(())
}

pub(crate) fn sanitize_language_code(raw: &str) -> Option<String> {
    let code = raw.trim();
    if code.is_empty() || code.len() > 32 {
        return None;
    }
    if code
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        Some(code.to_string())
    } else {
        None
    }
}

pub(crate) fn list_language_packs() -> Result<Vec<String>, String> {
    let packs = load_language_packs_checked()?;
    Ok(packs.into_iter().map(|p| p.code).collect())
}

pub(crate) fn read_language_pack(code: &str) -> Result<String, String> {
    let clean = sanitize_language_code(code).ok_or_else(|| "invalid language code".to_string())?;
    let packs = load_language_packs_checked()?;
    packs
        .into_iter()
        .find(|p| p.code == clean)
        .map(|p| p.content)
        .ok_or_else(|| "language not found".to_string())
}

fn languages_dir_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(LANGUAGES_DIR)
}

fn parse_language_pack_filename(name: &str) -> Option<(String, String)> {
    if !name.ends_with(".json") || !name.starts_with(LANGUAGE_PACK_PREFIX) {
        return None;
    }
    let stem = &name[LANGUAGE_PACK_PREFIX.len()..name.len() - 5];
    let (code_raw, ver_raw) = stem.rsplit_once('_')?;
    let code = sanitize_language_code(code_raw)?;
    if ver_raw.len() != 5 || !ver_raw.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((code, ver_raw.to_string()))
}

fn validate_language_pack_file(
    path: &Path,
    code: &str,
    ver: &str,
) -> Result<LoadedLanguagePack, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Language pack read failed: {} ({})", path.display(), e))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Language pack JSON invalid: {} ({})", path.display(), e))?;
    let obj = value
        .as_object()
        .ok_or_else(|| format!("Language pack root must be object: {}", path.display()))?;
    let meta = obj
        .get("__meta")
        .and_then(|v| v.as_object())
        .ok_or_else(|| format!("Language pack missing __meta object: {}", path.display()))?;

    let app = meta.get("app").and_then(|v| v.as_str()).unwrap_or_default();
    let meta_code = meta
        .get("language_code")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let meta_pack_ver = meta
        .get("pack_version")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let meta_inputer_ver = meta
        .get("inputer_version")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    if app != "KACF" {
        return Err(format!(
            "Language pack app mismatch: {} (expected KACF)",
            path.display()
        ));
    }
    if meta_code != code {
        return Err(format!(
            "Language pack code mismatch between filename and __meta: {}",
            path.display()
        ));
    }
    if meta_pack_ver != ver {
        return Err(format!(
            "Language pack version mismatch between filename and __meta: {}",
            path.display()
        ));
    }
    if meta_inputer_ver != LANGUAGE_INPUTER_VERSION {
        return Err(format!(
            "Language inputer version mismatch: pack={} software={} file={}",
            meta_inputer_ver,
            LANGUAGE_INPUTER_VERSION,
            path.display()
        ));
    }
    if meta_pack_ver != LANGUAGE_INPUTER_VERSION {
        return Err(format!(
            "Language pack version mismatch: pack={} software={} file={}",
            meta_pack_ver,
            LANGUAGE_INPUTER_VERSION,
            path.display()
        ));
    }

    let mut keys = BTreeSet::new();
    for (k, v) in obj {
        if k == "__meta" {
            continue;
        }
        if !v.is_string() {
            return Err(format!(
                "Language pack key must be string: file={} key={}",
                path.display(),
                k
            ));
        }
        keys.insert(k.to_string());
    }
    if keys.is_empty() {
        return Err(format!(
            "Language pack has no translation keys: {}",
            path.display()
        ));
    }

    Ok(LoadedLanguagePack {
        code: code.to_string(),
        path: path.to_path_buf(),
        content,
        keys,
    })
}

fn load_language_packs_checked() -> Result<Vec<LoadedLanguagePack>, String> {
    let mut items = Vec::new();
    let mut seen_codes = BTreeSet::new();
    let Ok(entries) = fs::read_dir(languages_dir_path()) else {
        return Err("Language pack directory missing".to_string());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|x| x.to_str()) else {
            continue;
        };
        if !name.ends_with(".json") {
            continue;
        }
        let Some((code, ver)) = parse_language_pack_filename(name) else {
            return Err(format!(
                "Language pack filename invalid (expected KACF_<code>_<ver>.json): {}",
                name
            ));
        };
        if !seen_codes.insert(code.clone()) {
            return Err(format!("Duplicate language code pack found: {}", code));
        }
        items.push(validate_language_pack_file(&path, &code, &ver)?);
    }
    if items.is_empty() {
        return Err("No valid language packs found".to_string());
    }
    let Some(base) = items.iter().find(|p| p.code == "en").cloned() else {
        return Err("Language packs must include English pack: code=en".to_string());
    };
    for pack in &items {
        if pack.keys != base.keys {
            return Err(format!(
                "Language pack keys incomplete or mismatched: {}",
                pack.path.display()
            ));
        }
    }
    items.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(items)
}

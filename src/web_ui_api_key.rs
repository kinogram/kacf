use std::fs;
use std::path::Path;

const API_KEY_SECRET_FILENAME: &str = ".kacf_api_key_secret.bin";
pub(crate) const API_KEY_ENC_PREFIX: &str = "kacfenc:v1:";

pub(crate) fn is_encrypted_api_key(value: &str) -> bool {
    value.starts_with(API_KEY_ENC_PREFIX)
}

pub(crate) fn mask_api_key_for_display(raw: &str) -> String {
    let v = raw.trim();
    if v.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = v.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!(
        "{}{}{}",
        head,
        "*".repeat(std::cmp::max(4, chars.len() - 8)),
        tail
    )
}

pub(crate) fn decrypt_shared_config_for_response(
    cfg: &mut crate::web_ui::SharedConfig,
    managed_root: &Path,
) {
    cfg.api_key_is_masked = false;
    if is_encrypted_api_key(&cfg.api_key) {
        match decrypt_api_key(&cfg.api_key, managed_root) {
            Some(plain) => cfg.api_key = plain,
            None => {
                eprintln!("[KACF] WARN: failed to decrypt api_key from shared_config");
                cfg.api_key.clear();
            }
        }
    }
}

pub(crate) fn encrypt_shared_config_for_storage(
    cfg: &mut crate::web_ui::SharedConfig,
    managed_root: &Path,
) {
    if !cfg.encrypt_api_key {
        if is_encrypted_api_key(&cfg.api_key) {
            if let Some(plain) = decrypt_api_key(&cfg.api_key, managed_root) {
                cfg.api_key = plain;
            }
        }
        return;
    }
    if cfg.api_key.trim().is_empty() {
        return;
    }
    if is_encrypted_api_key(&cfg.api_key) {
        return;
    }
    if let Some(enc) = encrypt_api_key(&cfg.api_key, managed_root) {
        cfg.api_key = enc;
    } else {
        eprintln!("[KACF] WARN: failed to encrypt api_key; storing plaintext this round");
    }
}

fn api_key_secret_path(managed_root: &Path) -> std::path::PathBuf {
    managed_root.join(API_KEY_SECRET_FILENAME)
}

fn random_bytes(len: usize, fallback_seed: u64) -> Vec<u8> {
    let mut out = vec![0u8; len];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        use std::io::Read;
        if f.read_exact(&mut out).is_ok() {
            return out;
        }
    }
    // Fallback for environments without /dev/urandom.
    let mut x = fallback_seed ^ 0x9e37_79b9_7f4a_7c15;
    for b in &mut out {
        x ^= x << 7;
        x ^= x >> 9;
        x ^= x << 8;
        *b = (x & 0xff) as u8;
    }
    out
}

fn load_or_create_api_key_secret(managed_root: &Path) -> std::io::Result<Vec<u8>> {
    let path = api_key_secret_path(managed_root);
    if let Ok(raw) = fs::read(&path) {
        if raw.len() >= 16 {
            return Ok(raw);
        }
    }
    fs::create_dir_all(managed_root)?;
    let now_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secret = random_bytes(32, now_seed);
    fs::write(&path, &secret)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(secret)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let h = chars[i].to_digit(16)?;
        let l = chars[i + 1].to_digit(16)?;
        out.push(((h << 4) | l) as u8);
        i += 2;
    }
    Some(out)
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Clone)]
struct XorShift128Plus {
    s0: u64,
    s1: u64,
}

impl XorShift128Plus {
    fn new(seed: &[u8], nonce: &[u8]) -> Self {
        let mut b = Vec::with_capacity(seed.len() + nonce.len() + 4);
        b.extend_from_slice(seed);
        b.extend_from_slice(nonce);
        b.extend_from_slice(b"s0");
        let mut s0 = fnv1a64(&b);
        b.truncate(seed.len() + nonce.len());
        b.extend_from_slice(b"s1");
        let mut s1 = fnv1a64(&b);
        if s0 == 0 && s1 == 0 {
            s0 = 0x6a09e667f3bcc909;
            s1 = 0xbb67ae8584caa73b;
        }
        Self { s0, s1 }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.s0;
        let y = self.s1;
        self.s0 = y;
        x ^= x << 23;
        self.s1 = x ^ y ^ (x >> 17) ^ (y >> 26);
        self.s1.wrapping_add(y)
    }
}

fn xor_stream_crypt(seed: &[u8], nonce: &[u8], data: &[u8]) -> Vec<u8> {
    let mut rng = XorShift128Plus::new(seed, nonce);
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0usize;
    while i < data.len() {
        let block = rng.next_u64().to_le_bytes();
        for b in block {
            if i >= data.len() {
                break;
            }
            out.push(data[i] ^ b);
            i += 1;
        }
    }
    out
}

fn encrypt_api_key(plain: &str, managed_root: &Path) -> Option<String> {
    if plain.trim().is_empty() {
        return Some(String::new());
    }
    let secret = load_or_create_api_key_secret(managed_root).ok()?;
    let now_seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let nonce = random_bytes(12, now_seed);
    let cipher = xor_stream_crypt(&secret, &nonce, plain.as_bytes());
    Some(format!(
        "{}{}:{}",
        API_KEY_ENC_PREFIX,
        hex_encode(&nonce),
        hex_encode(&cipher)
    ))
}

fn decrypt_api_key(enc: &str, managed_root: &Path) -> Option<String> {
    let tail = enc.strip_prefix(API_KEY_ENC_PREFIX)?;
    let (nonce_hex, cipher_hex) = tail.split_once(':')?;
    let nonce = hex_decode(nonce_hex)?;
    if nonce.is_empty() {
        return None;
    }
    let cipher = hex_decode(cipher_hex)?;
    let secret = load_or_create_api_key_secret(managed_root).ok()?;
    let plain = xor_stream_crypt(&secret, &nonce, &cipher);
    String::from_utf8(plain).ok()
}

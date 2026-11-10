//! Local root CA and per-site certificates (rcgen in-process; cached on disk).

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::Paths;
use crate::error::{DevStackError, Result};

pub struct CertPaths {
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub site_cert: PathBuf,
    pub site_key: PathBuf,
}

pub fn ensure_root_ca(paths: &Paths) -> Result<(PathBuf, PathBuf)> {
    let ca_dir = paths.certs.join("ca");
    fs::create_dir_all(&ca_dir)?;
    let ca_cert = ca_dir.join("devstack-root.crt");
    let ca_key = ca_dir.join("devstack-root.key");
    if ca_cert.exists() && ca_key.exists() {
        return Ok((ca_cert, ca_key));
    }
    // Always mint with rcgen — spawning openssl is slow and breaks on Windows LibreSSL.
    generate_ca_rcgen(&ca_cert, &ca_key)?;
    Ok((ca_cert, ca_key))
}

pub fn ensure_site_cert(paths: &Paths, domain: &str) -> Result<CertPaths> {
    ensure_site_cert_with_sans(paths, domain, &[])
}

/// Certificate covering the site's domain, aliases and (optionally) `*.host` wildcards.
pub fn ensure_site_cert_for(paths: &Paths, site: &crate::config::SiteRecord) -> Result<CertPaths> {
    let extra: Vec<String> = site
        .server_names()
        .into_iter()
        .filter(|n| *n != site.domain)
        .collect();
    ensure_site_cert_with_sans(paths, &site.domain, &extra)
}

/// Stable fingerprint of the extra SAN list (sorted, de-duplicated, lowercase).
pub fn sans_fingerprint(extra: &[String]) -> String {
    let mut v: Vec<String> = extra
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v.join(",")
}

pub fn ensure_site_cert_with_sans(
    paths: &Paths,
    domain: &str,
    extra_sans: &[String],
) -> Result<CertPaths> {
    let (ca_cert, ca_key) = ensure_root_ca(paths)?;
    let site_dir = paths.certs.join("sites").join(domain);
    fs::create_dir_all(&site_dir)?;
    let site_cert = site_dir.join("fullchain.pem");
    let site_key = site_dir.join("privkey.pem");
    let sans_marker = site_dir.join("sans.txt");
    let want = sans_fingerprint(extra_sans);
    let have = fs::read_to_string(&sans_marker).unwrap_or_default();
    if site_cert.exists() && site_key.exists() && have.trim() == want {
        return Ok(CertPaths {
            ca_cert,
            ca_key,
            site_cert,
            site_key,
        });
    }
    generate_site_rcgen(&ca_cert, &ca_key, &site_cert, &site_key, domain, extra_sans)?;
    let _ = fs::write(&sans_marker, &want);
    Ok(CertPaths {
        ca_cert,
        ca_key,
        site_cert,
        site_key,
    })
}

pub fn ca_cert_path(paths: &Paths) -> PathBuf {
    paths.certs.join("ca/devstack-root.crt")
}

fn ca_trust_marker(paths: &Paths) -> PathBuf {
    paths.certs.join("ca/.trusted-sha256")
}

/// Set when the user declined / timed out UAC (or CI skipped elevation) so
/// apply_stack does not re-prompt on every warm start. Cleared by Retry.
fn ca_trust_deferred_marker(paths: &Paths) -> PathBuf {
    paths.certs.join("ca/.trust-deferred")
}

fn ca_sha256(paths: &Paths) -> Result<String> {
    let bytes = fs::read(ca_cert_path(paths))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()))
}

/// Parse `Cert Hash(sha1): aabb...` lines from `certutil -dump` output.
pub fn parse_certutil_sha1_hash(dump: &str) -> Option<String> {
    for line in dump.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(idx) = lower.find("cert hash(sha1):") else {
            continue;
        };
        let raw = line[idx + "cert hash(sha1):".len()..].trim();
        let hex: String = raw
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect::<String>()
            .to_ascii_lowercase();
        if hex.len() == 40 {
            return Some(hex);
        }
    }
    None
}

/// True when a `certutil -store` listing contains the given SHA1 thumbprint.
pub fn store_output_contains_hash(store_out: &str, hash: &str) -> bool {
    let want = hash
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    if want.len() != 40 {
        return false;
    }
    let compact = store_out
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.contains(&want)
}

/// Probe CurrentUser and LocalMachine Root stores for our CA (Windows).
/// Non-Windows: check helper-installed locations / marker only after verify helpers.
pub fn is_ca_in_trust_store(paths: &Paths) -> bool {
    let ca = ca_cert_path(paths);
    if !ca.exists() {
        return false;
    }
    #[cfg(windows)]
    {
        return is_ca_in_trust_store_windows(&ca);
    }
    #[cfg(target_os = "macos")]
    {
        return is_ca_in_trust_store_macos(&ca, paths);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return is_ca_in_trust_store_linux(paths);
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = paths;
        false
    }
}

#[cfg(windows)]
fn is_ca_in_trust_store_windows(ca: &Path) -> bool {
    let dump = crate::services::process::command("certutil")
        .args(["-dump", &ca.display().to_string()])
        .output()
        .ok();
    let Some(dump) = dump else {
        return false;
    };
    let text = String::from_utf8_lossy(&dump.stdout);
    let Some(hash) = parse_certutil_sha1_hash(&text) else {
        return false;
    };
    for user_flag in [false, true] {
        let mut cmd = crate::services::process::command("certutil");
        if user_flag {
            cmd.arg("-user");
        }
        let out = cmd.args(["-store", "Root", &hash]).output();
        if let Ok(o) = out {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            if o.status.success() && store_output_contains_hash(&combined, &hash) {
                return true;
            }
            // certutil -store Root <hash> succeeds with the cert body when present.
            if o.status.success() && combined.to_ascii_lowercase().contains("devstack local ca") {
                return true;
            }
            if o.status.success() && !combined.to_ascii_lowercase().contains("element not found")
            {
                // Hash lookup returned a cert entry.
                if combined.to_ascii_lowercase().contains("serial")
                    || combined.to_ascii_lowercase().contains("issuer")
                {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(target_os = "macos")]
fn is_ca_in_trust_store_macos(ca: &Path, paths: &Paths) -> bool {
    let _ = ca;
    let Ok(want) = ca_sha256(paths) else {
        return false;
    };
    let out = crate::services::process::command("security")
        .args([
            "find-certificate",
            "-a",
            "-c",
            "DevStack Local CA",
            "-Z",
            "/Library/Keychains/System.keychain",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            text.contains(&want) || text.contains("devstack local ca")
        }
        _ => false,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn is_ca_in_trust_store_linux(paths: &Paths) -> bool {
    let installed = Path::new("/usr/local/share/ca-certificates/devstack-root.crt");
    if !installed.exists() {
        return false;
    }
    let Ok(want) = ca_sha256(paths) else {
        return false;
    };
    let mut h = Sha256::new();
    if let Ok(bytes) = fs::read(installed) {
        h.update(&bytes);
        return hex::encode(h.finalize()) == want;
    }
    false
}

/// True only when the CA is verified present in the OS trust store.
/// A on-disk marker alone never counts as trusted.
pub fn is_ca_trusted(paths: &Paths) -> bool {
    if !is_ca_in_trust_store(paths) {
        // Stale marker from older builds that marked trust without store proof.
        let _ = clear_ca_trusted_marker(paths);
        return false;
    }
    // Refresh marker so warm paths can short-circuit hosts+CA when both ok.
    let _ = mark_ca_trusted(paths);
    true
}

/// Legacy name — now requires store verification (marker alone is insufficient).
pub fn is_ca_trust_cached(paths: &Paths) -> bool {
    is_ca_trusted(paths)
}

/// Record trust **only after** `is_ca_in_trust_store` is true.
pub fn mark_ca_trusted(paths: &Paths) -> Result<()> {
    if !is_ca_in_trust_store(paths) {
        return Err(DevStackError::Message(
            "refusing to mark CA trusted: certificate not found in CurrentUser/LocalMachine Root (or system store)"
                .into(),
        ));
    }
    let hash = ca_sha256(paths)?;
    if let Some(parent) = ca_trust_marker(paths).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(ca_trust_marker(paths), hash)?;
    clear_ca_trust_deferred(paths);
    Ok(())
}

pub fn clear_ca_trusted_marker(paths: &Paths) -> Result<()> {
    let p = ca_trust_marker(paths);
    if p.exists() {
        fs::remove_file(&p)?;
    }
    Ok(())
}

pub fn is_ca_trust_deferred(paths: &Paths) -> bool {
    ca_trust_deferred_marker(paths).exists()
}

pub fn mark_ca_trust_deferred(paths: &Paths) -> Result<()> {
    if let Some(parent) = ca_trust_deferred_marker(paths).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(ca_trust_deferred_marker(paths), "1")?;
    Ok(())
}

pub fn clear_ca_trust_deferred(paths: &Paths) {
    let _ = fs::remove_file(ca_trust_deferred_marker(paths));
}

/// User-facing notice when the CA is not in the trust store.
pub fn ca_untrusted_message() -> &'static str {
    "HTTPS certificate not trusted — browsers will warn until the DevStack CA is installed. Click Retry to elevate and trust it."
}

pub fn trust_ca_via_helper(paths: &Paths) -> Result<()> {
    let ca = ca_cert_path(paths);
    if !ca.exists() {
        ensure_root_ca(paths)?;
    }
    if is_ca_trusted(paths) {
        return Ok(());
    }
    crate::services::dns::helper_run(paths, &["trust-ca", &ca.display().to_string()])?;
    if !is_ca_in_trust_store(paths) {
        return Err(DevStackError::Message(
            "CA trust helper finished but the certificate is not in the system trust store".into(),
        ));
    }
    mark_ca_trusted(paths)?;
    Ok(())
}

fn generate_ca_rcgen(ca_cert: &Path, ca_key: &Path) -> Result<()> {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
    let mut params = CertificateParams::new(vec!["DevStack Local CA".into()])
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    params
        .distinguished_name
        .push(DnType::CommonName, "DevStack Local CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let key = KeyPair::generate().map_err(|e| DevStackError::Message(e.to_string()))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    fs::write(ca_cert, cert.pem())?;
    fs::write(ca_key, key.serialize_pem())?;
    Ok(())
}

fn generate_site_rcgen(
    ca_cert: &Path,
    ca_key: &Path,
    site_cert: &Path,
    site_key: &Path,
    domain: &str,
    extra_sans: &[String],
) -> Result<()> {
    use rcgen::{CertificateParams, DnType, KeyPair, SanType};

    let ca_pem = fs::read_to_string(ca_cert)?;
    let ca_key_pem = fs::read_to_string(ca_key)?;
    let ca_key_pair =
        KeyPair::from_pem(&ca_key_pem).map_err(|e| DevStackError::Message(e.to_string()))?;
    let ca_params = CertificateParams::from_ca_cert_pem(&ca_pem)
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    // Reconstruct issuer Certificate (preserves SKI via PreSpecified from PEM).
    let issuer = ca_params
        .self_signed(&ca_key_pair)
        .map_err(|e| DevStackError::Message(e.to_string()))?;

    let mut params = CertificateParams::new(vec![domain.to_string()])
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    params
        .distinguished_name
        .push(DnType::CommonName, domain);
    params.subject_alt_names = vec![
        SanType::DnsName(
            domain
                .try_into()
                .map_err(|e: rcgen::Error| DevStackError::Message(e.to_string()))?,
        ),
        SanType::DnsName(
            "localhost"
                .try_into()
                .map_err(|e: rcgen::Error| DevStackError::Message(e.to_string()))?,
        ),
    ];
    for extra in sans_fingerprint(extra_sans).split(',').filter(|s| !s.is_empty()) {
        if extra == domain || extra == "localhost" {
            continue;
        }
        params.subject_alt_names.push(SanType::DnsName(
            extra
                .try_into()
                .map_err(|e: rcgen::Error| DevStackError::Message(e.to_string()))?,
        ));
    }
    let key = KeyPair::generate().map_err(|e| DevStackError::Message(e.to_string()))?;
    let cert = params
        .signed_by(&key, &issuer, &ca_key_pair)
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    fs::write(site_cert, format!("{}{}", cert.pem(), ca_pem))?;
    fs::write(site_key, key.serialize_pem())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn site_cert_is_cached_and_fast() {
        let dir = std::env::temp_dir().join(format!(
            "devstack-tls-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let t0 = Instant::now();
        ensure_site_cert(&paths, "cache.test").unwrap();
        let first = t0.elapsed();
        let t1 = Instant::now();
        ensure_site_cert(&paths, "cache.test").unwrap();
        let second = t1.elapsed();
        assert!(
            second.as_millis() < 50,
            "cached cert should be near-instant, took {second:?} (first {first:?})"
        );
        assert!(!is_ca_trust_cached(&paths));
        // Marker-only must never count as trusted without store verification.
        let _ = fs::write(ca_trust_marker(&paths), "deadbeef");
        assert!(!is_ca_trusted(&paths));
        let err = mark_ca_trusted(&paths).unwrap_err().to_string();
        assert!(
            err.contains("refusing to mark CA trusted"),
            "mark without store must fail: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn certutil_hash_parsers_roundtrip() {
        let dump = "\
Cert Hash(md5): 11 22 33\r\n\
Cert Hash(sha1): Aa Bb Cc Dd Ee Ff 01 23 45 67 89 Ab Cd Ef 01 23 45 67 89 Ab\r\n\
Cert Hash(sha256): deadbeef\r\n";
        let hash = parse_certutil_sha1_hash(dump).expect("hash");
        assert_eq!(hash, "aabbccddeeff0123456789abcdef0123456789ab");
        assert!(store_output_contains_hash(
            "================ Certificate 0 ================\r\nCert Hash(sha1): aa bb cc dd ee ff 01 23 45 67 89 ab cd ef 01 23 45 67 89 ab\r\n",
            &hash
        ));
        assert!(!store_output_contains_hash("empty", &hash));
    }

    #[test]
    fn deferred_marker_blocks_reprompt_until_cleared() {
        let dir = std::env::temp_dir().join(format!("devstack-tls-def-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        ensure_root_ca(&paths).unwrap();
        assert!(!is_ca_trust_deferred(&paths));
        mark_ca_trust_deferred(&paths).unwrap();
        assert!(is_ca_trust_deferred(&paths));
        clear_ca_trust_deferred(&paths);
        assert!(!is_ca_trust_deferred(&paths));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cert_regenerates_when_aliases_change() {
        let dir = std::env::temp_dir().join(format!("devstack-tls-alias-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let a = ensure_site_cert_with_sans(&paths, "app.test", &[]).unwrap();
        let first = fs::read_to_string(&a.site_cert).unwrap();
        let same = ensure_site_cert_with_sans(&paths, "app.test", &[]).unwrap();
        assert_eq!(first, fs::read_to_string(&same.site_cert).unwrap());
        let extra = vec!["shop.test".to_string(), "*.app.test".to_string()];
        let b = ensure_site_cert_with_sans(&paths, "app.test", &extra).unwrap();
        assert_ne!(first, fs::read_to_string(&b.site_cert).unwrap());
        assert_eq!(
            fs::read_to_string(dir.join("certs/sites/app.test/sans.txt")).unwrap(),
            "*.app.test,shop.test"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sans_fingerprint_is_order_insensitive() {
        assert_eq!(
            sans_fingerprint(&["b.test".into(), "A.test".into(), "b.test".into()]),
            "a.test,b.test"
        );
    }
}

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

fn ca_sha256(paths: &Paths) -> Result<String> {
    let bytes = fs::read(ca_cert_path(paths))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()))
}

/// True when we previously recorded a successful trust of the current CA file.
pub fn is_ca_trust_cached(paths: &Paths) -> bool {
    let Ok(hash) = ca_sha256(paths) else {
        return false;
    };
    fs::read_to_string(ca_trust_marker(paths))
        .map(|s| s.trim() == hash)
        .unwrap_or(false)
}

pub fn mark_ca_trusted(paths: &Paths) -> Result<()> {
    let hash = ca_sha256(paths)?;
    if let Some(parent) = ca_trust_marker(paths).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(ca_trust_marker(paths), hash)?;
    Ok(())
}

pub fn trust_ca_via_helper(paths: &Paths) -> Result<()> {
    let ca = ca_cert_path(paths);
    if !ca.exists() {
        ensure_root_ca(paths)?;
    }
    if is_ca_trust_cached(paths) {
        return Ok(());
    }
    crate::services::dns::helper_run(paths, &["trust-ca", &ca.display().to_string()])?;
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
        assert!(is_ca_trust_cached(&paths) == false);
        mark_ca_trusted(&paths).unwrap();
        assert!(is_ca_trust_cached(&paths));
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

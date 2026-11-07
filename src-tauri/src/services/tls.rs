//! Local root CA and per-site certificates (OpenSSL CLI, rcgen fallback).

use std::fs;
use std::path::{Path, PathBuf};

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

    // Windows LibreSSL/OpenSSL treats `/CN=…` as a filesystem path and mangles
    // `//CN=` — use rcgen there. Unix keeps openssl when available.
    if !cfg!(windows) && which::which("openssl").is_ok() {
        run_openssl(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-sha256",
            "-days",
            "3650",
            "-nodes",
            "-keyout",
            &ca_key.to_string_lossy(),
            "-out",
            &ca_cert.to_string_lossy(),
            "-subj",
            "/CN=DevStack Local CA/O=DevStack",
        ])?;
        return Ok((ca_cert, ca_key));
    }

    generate_ca_rcgen(&ca_cert, &ca_key)?;
    Ok((ca_cert, ca_key))
}

pub fn ensure_site_cert(paths: &Paths, domain: &str) -> Result<CertPaths> {
    let (ca_cert, ca_key) = ensure_root_ca(paths)?;
    let site_dir = paths.certs.join("sites").join(domain);
    fs::create_dir_all(&site_dir)?;
    let site_cert = site_dir.join("fullchain.pem");
    let site_key = site_dir.join("privkey.pem");
    if site_cert.exists() && site_key.exists() {
        return Ok(CertPaths {
            ca_cert,
            ca_key,
            site_cert,
            site_key,
        });
    }

    if !cfg!(windows) && which::which("openssl").is_ok() {
        let csr = site_dir.join("request.csr");
        let ext = site_dir.join("ext.cnf");
        fs::write(
            &ext,
            format!(
                "basicConstraints=CA:FALSE\nsubjectAltName=DNS:{domain},DNS:localhost\n"
            ),
        )?;
        let site_subj = format!("/CN={domain}");
        run_openssl(&[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            &site_key.to_string_lossy(),
            "-out",
            &csr.to_string_lossy(),
            "-subj",
            &site_subj,
        ])?;
        let leaf = site_dir.join("leaf.crt");
        run_openssl(&[
            "x509",
            "-req",
            "-in",
            &csr.to_string_lossy(),
            "-CA",
            &ca_cert.to_string_lossy(),
            "-CAkey",
            &ca_key.to_string_lossy(),
            "-CAcreateserial",
            "-out",
            &leaf.to_string_lossy(),
            "-days",
            "825",
            "-sha256",
            "-extfile",
            &ext.to_string_lossy(),
        ])?;
        let leaf_pem = fs::read_to_string(&leaf)?;
        let ca_pem = fs::read_to_string(&ca_cert)?;
        fs::write(&site_cert, format!("{leaf_pem}{ca_pem}"))?;
        let _ = fs::remove_file(csr);
        return Ok(CertPaths {
            ca_cert,
            ca_key,
            site_cert,
            site_key,
        });
    }

    generate_site_rcgen(&ca_cert, &ca_key, &site_cert, &site_key, domain)?;
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

pub fn trust_ca_via_helper(paths: &Paths) -> Result<()> {
    let ca = ca_cert_path(paths);
    if !ca.exists() {
        ensure_root_ca(paths)?;
    }
    crate::services::dns::helper_run(paths, &["trust-ca", &ca.display().to_string()])
}

fn run_openssl(args: &[&str]) -> Result<()> {
    let status = crate::services::process::command("openssl")
        .args(args)
        .status()
        .map_err(|e| DevStackError::Message(format!("openssl: {e}")))?;
    if !status.success() {
        return Err(DevStackError::Message(format!(
            "openssl {:?} failed",
            args
        )));
    }
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
) -> Result<()> {
    use rcgen::{CertificateParams, DnType, KeyPair, SanType};
    // Without a full Issuer API dance, mint a standalone cert for local smoke
    // when openssl is unavailable. Browsers will warn; curl -k still works.
    let _ = (ca_cert, ca_key);
    let mut params = CertificateParams::new(vec![domain.to_string()])
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    params
        .distinguished_name
        .push(DnType::CommonName, domain);
    params.subject_alt_names = vec![SanType::DnsName(
        domain
            .try_into()
            .map_err(|e: rcgen::Error| DevStackError::Message(e.to_string()))?,
    )];
    let key = KeyPair::generate().map_err(|e| DevStackError::Message(e.to_string()))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    fs::write(site_cert, cert.pem())?;
    fs::write(site_key, key.serialize_pem())?;
    Ok(())
}

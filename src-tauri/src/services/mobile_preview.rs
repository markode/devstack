//! Mobile preview: expose one site on the LAN (`http://<lan-ip>:<pool-port>`) so a phone
//! on the same Wi-Fi can open it. An extra nginx server block listens on the LAN IP and
//! proxies to the site's real vhost with `Host: <site domain>`; WordPress `siteurl/home`
//! are rewritten to the preview URL while active and restored on stop.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::Path;

use serde::Serialize;

use crate::config::{AppState, MobilePreviewState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::ports::port_is_free;

pub const PREVIEW_PORT_RANGE: std::ops::RangeInclusive<u16> = 8300..=8399;
pub const CONF_NAME: &str = "zz-mobile-preview.conf";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobilePreviewInfo {
    pub active: bool,
    pub site_id: Option<String>,
    pub lan_ip: Option<String>,
    pub port: Option<u16>,
    pub url: Option<String>,
    /// `data:image/svg+xml;base64,…` QR code for `url`.
    pub qr_data_url: Option<String>,
    pub host_header: Option<String>,
    pub wp_rewritten: bool,
    pub firewall_notice: Option<String>,
    pub notes: Vec<String>,
}

pub fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.octets()[0] == 169 && ip.octets()[1] == 254
}

/// Best-effort LAN address: the interface the OS would route a public address through.
/// (A UDP `connect` sends no packets.)
pub fn detect_lan_ip() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("8.8.8.8", 80)).ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

pub fn preview_url(ip: &str, port: u16) -> String {
    format!("http://{ip}:{port}")
}

pub fn pick_port() -> Option<u16> {
    PREVIEW_PORT_RANGE.clone().find(|p| port_is_free(*p))
}

pub fn firewall_notice_for(os: &str, port: u16) -> String {
    match os {
        "windows" => format!(
            "Windows Firewall may block phones: run once in an admin terminal — netsh advfirewall firewall add rule name=\"DevStack preview\" dir=in action=allow protocol=TCP localport={port}"
        ),
        "macos" => format!(
            "macOS may ask to allow incoming connections for nginx — click Allow (or System Settings → Network → Firewall). Port {port} must be reachable from your phone."
        ),
        _ => format!(
            "If a firewall is active allow the port, e.g. `sudo ufw allow {port}/tcp`. Your phone must be on the same network."
        ),
    }
}

pub fn firewall_notice(port: u16) -> String {
    firewall_notice_for(std::env::consts::OS, port)
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Render `text` as an SVG QR code data URL.
pub fn qr_data_url(text: &str) -> Result<String> {
    let code = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| DevStackError::Message(format!("QR encode failed: {e}")))?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .build();
    Ok(format!("data:image/svg+xml;base64,{}", b64(svg.as_bytes())))
}

fn backend_port(site: &SiteRecord, state: &AppState) -> (u16, bool) {
    let apache = crate::services::httpd::site_uses_apache(site);
    if site.https {
        (if apache { state.stack.apache_https_port } else { state.stack.https_port }, true)
    } else {
        (if apache { state.stack.apache_http_port } else { state.stack.http_port }, false)
    }
}

/// nginx server block that fronts the site on `lan_ip:port`.
pub fn nginx_conf(lan_ip: &str, port: u16, site: &SiteRecord, state: &AppState) -> String {
    let (backend, tls) = backend_port(site, state);
    let scheme = if tls { "https" } else { "http" };
    let ssl_lines = if tls {
        format!("    proxy_ssl_server_name on;\n    proxy_ssl_name {};\n    proxy_ssl_verify off;\n", site.domain)
    } else {
        String::new()
    };
    let domain_re = site.domain.replace('.', "\\.");
    format!(
        r#"# DevStack mobile preview for {domain} — managed, removed on stop
server {{
  listen {lan_ip}:{port};
  server_name _;
  client_max_body_size 0;
  location / {{
    proxy_pass {scheme}://127.0.0.1:{backend};
    proxy_http_version 1.1;
    proxy_set_header Host {domain};
    proxy_set_header X-Forwarded-Host {lan_ip}:{port};
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Forwarded-Proto http;
{ssl_lines}    proxy_redirect ~^https?://{domain_re}(:\d+)?(/.*)?$ http://{lan_ip}:{port}$2;
    proxy_read_timeout 3600s;
  }}
}}
"#,
        domain = site.domain,
    )
}

/// Called by the nginx renderer so a reload never drops an active preview.
pub fn write_nginx_conf_if_active(paths: &Paths, state: &AppState, sites_dir: &Path) {
    let Some(mp) = state.runtime.mobile_preview.as_ref() else { return };
    let Some(site) = state.sites.iter().find(|s| s.id == mp.site_id) else { return };
    let _ = fs::create_dir_all(sites_dir);
    let _ = fs::write(sites_dir.join(CONF_NAME), nginx_conf(&mp.lan_ip, mp.port, site, state));
    let _ = paths;
}

pub fn status(state: &AppState) -> MobilePreviewInfo {
    match state.runtime.mobile_preview.as_ref() {
        Some(mp) => {
            let domain = state.sites.iter().find(|s| s.id == mp.site_id).map(|s| s.domain.clone());
            MobilePreviewInfo {
                active: true,
                site_id: Some(mp.site_id.clone()),
                lan_ip: Some(mp.lan_ip.clone()),
                port: Some(mp.port),
                url: Some(mp.url.clone()),
                qr_data_url: qr_data_url(&mp.url).ok(),
                host_header: domain,
                wp_rewritten: mp.original_wp_url.is_some(),
                firewall_notice: Some(firewall_notice(mp.port)),
                notes: vec![],
            }
        }
        None => MobilePreviewInfo {
            active: false,
            site_id: None,
            lan_ip: detect_lan_ip().map(|i| i.to_string()),
            port: None,
            url: None,
            qr_data_url: None,
            host_header: None,
            wp_rewritten: false,
            firewall_notice: None,
            notes: vec![],
        },
    }
}

pub fn start(paths: &Paths, state: &mut AppState, site_id: &str) -> Result<MobilePreviewInfo> {
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| DevStackError::Message(format!("site not found: {site_id}")))?;
    let nginx_up = state.runtime.nginx_pid.map(crate::services::process::is_running).unwrap_or(false);
    if !nginx_up {
        return Err(DevStackError::Message(
            "Start the stack first — mobile preview is served by Nginx.".into(),
        ));
    }
    if state.runtime.mobile_preview.is_some() {
        stop(paths, state)?;
    }
    let lan_ip = detect_lan_ip()
        .ok_or_else(|| DevStackError::Message("No LAN address found — connect to Wi-Fi/Ethernet first.".into()))?
        .to_string();
    let port = pick_port()
        .ok_or_else(|| DevStackError::Message("No free preview port in 8300-8399".into()))?;
    let url = preview_url(&lan_ip, port);
    state.runtime.mobile_preview = Some(MobilePreviewState {
        site_id: site.id.clone(),
        lan_ip: lan_ip.clone(),
        port,
        url: url.clone(),
        original_wp_url: None,
    });
    if let Err(e) = crate::services::nginx::reload_nginx(paths, state) {
        state.runtime.mobile_preview = None;
        return Err(e);
    }

    let mut notes = Vec::new();
    if crate::services::wp_url::site_is_wordpress(&site) && site.database_name.is_some() {
        let canonical = crate::services::ports::canonical_site_url(&site, state);
        match crate::services::wp_url::repair_site_url_to(paths, state, &site, &url, Some(&canonical), None) {
            Ok(_) => {
                if let Some(mp) = state.runtime.mobile_preview.as_mut() {
                    mp.original_wp_url = Some(canonical);
                }
            }
            Err(e) => notes.push(format!("WordPress URL rewrite skipped: {e}")),
        }
    }
    crate::config::save_state(&paths.state_file, state)?;
    let mut info = status(state);
    info.notes = notes;
    Ok(info)
}

pub fn stop(paths: &Paths, state: &mut AppState) -> Result<MobilePreviewInfo> {
    let Some(mp) = state.runtime.mobile_preview.take() else {
        return Ok(status(state));
    };
    let mut notes = Vec::new();
    let _ = fs::remove_file(paths.nginx_sites_dir().join(CONF_NAME));
    if let Some(original) = mp.original_wp_url.as_deref() {
        if let Some(site) = state.sites.iter().find(|s| s.id == mp.site_id).cloned() {
            if let Err(e) = crate::services::wp_url::repair_site_url_to(paths, state, &site, original, Some(&mp.url), None) {
                notes.push(format!("Could not restore WordPress URL ({e}) — use Repair site URL."));
            }
        }
    }
    let nginx_up = state.runtime.nginx_pid.map(crate::services::process::is_running).unwrap_or(false);
    if nginx_up {
        let _ = crate::services::nginx::reload_nginx(paths, state);
    }
    crate::config::save_state(&paths.state_file, state)?;
    let mut info = status(state);
    info.notes = notes;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn site(https: bool) -> SiteRecord {
        let mut s = SiteRecord {
            id: "blog".into(),
            name: "blog".into(),
            path: "/tmp/blog".into(),
            domain: "blog.test".into(),
            php_version: None,
            node_version: None,
            https,
            web_server: Some("nginx".into()),
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: String::new(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: vec![],
            wildcard: false,
        };
        s.ensure_domain();
        s
    }

    #[test]
    fn url_builder_and_private_ranges() {
        assert_eq!(preview_url("192.168.1.20", 8300), "http://192.168.1.20:8300");
        assert!(is_private_ipv4("192.168.0.5".parse().unwrap()));
        assert!(is_private_ipv4("10.1.2.3".parse().unwrap()));
        assert!(is_private_ipv4("172.16.0.1".parse().unwrap()));
        assert!(!is_private_ipv4("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn nginx_conf_sets_host_header_and_binds_lan_ip() {
        let st = AppState::default();
        let https = nginx_conf("192.168.1.20", 8300, &site(true), &st);
        assert!(https.contains("listen 192.168.1.20:8300;"));
        assert!(https.contains("proxy_set_header Host blog.test;"));
        assert!(https.contains("proxy_pass https://127.0.0.1:443;"));
        assert!(https.contains("proxy_ssl_name blog.test;"));
        assert!(https.contains("blog\\.test"));
        let http = nginx_conf("10.0.0.2", 8301, &site(false), &st);
        assert!(http.contains("proxy_pass http://127.0.0.1:80;"));
        assert!(!http.contains("proxy_ssl_name"));
        assert!(!http.contains("sub_filter"), "sub_filter needs an optional nginx module");
    }

    #[test]
    fn qr_data_url_is_svg_base64() {
        let u = qr_data_url("http://192.168.1.20:8300").unwrap();
        assert!(u.starts_with("data:image/svg+xml;base64,"));
        assert!(u.len() > 200);
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
        assert_eq!(b64(b"M"), "TQ==");
    }

    #[test]
    fn firewall_notice_mentions_port_per_os() {
        assert!(firewall_notice_for("windows", 8300).contains("netsh advfirewall"));
        assert!(firewall_notice_for("linux", 8300).contains("8300"));
        assert!(firewall_notice_for("macos", 8300).contains("Allow"));
    }

    #[test]
    fn pick_port_skips_busy_ports() {
        let first = pick_port().unwrap();
        assert!(PREVIEW_PORT_RANGE.contains(&first));
        if let Ok(_holder) = TcpListener::bind(("0.0.0.0", first)) {
            let second = pick_port().unwrap();
            assert_ne!(first, second);
        }
    }

    #[test]
    fn start_requires_running_nginx_and_stop_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("devstack-mp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let mut st = AppState::default();
        st.sites.push(site(true));
        let err = start(&paths, &mut st, "blog").unwrap_err().to_string();
        assert!(err.contains("Start the stack"));
        assert!(start(&paths, &mut st, "nope").is_err());
        let info = stop(&paths, &mut st).unwrap();
        assert!(!info.active);
        // Active state is reflected in status + survives conf rendering.
        st.runtime.mobile_preview = Some(MobilePreviewState {
            site_id: "blog".into(),
            lan_ip: "192.168.1.20".into(),
            port: 8305,
            url: preview_url("192.168.1.20", 8305),
            original_wp_url: None,
        });
        let info = status(&st);
        assert!(info.active && info.qr_data_url.is_some());
        assert_eq!(info.host_header.as_deref(), Some("blog.test"));
        write_nginx_conf_if_active(&paths, &st, &paths.nginx_sites_dir());
        assert!(paths.nginx_sites_dir().join(CONF_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}

//! Site detection (WordPress / Laravel) and detail helpers.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;

use crate::config::{slugify, AppState, Paths, SiteRecord};
use crate::error::Result;
use crate::services::nginx::expand_home;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteDetails {
    pub site: SiteRecord,
    pub framework: String,
    pub framework_icon: String,
    pub absolute_path: PathBuf,
    pub url: String,
    pub https_url: String,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub phpmyadmin_url: Option<String>,
    pub has_wp_config: bool,
}

pub fn detect_framework(path: &Path) -> (String, String) {
    let root = expand_home(path);
    if root.join("wp-config.php").exists() || root.join("wp-config-sample.php").exists() {
        return ("wordpress".into(), "WP".into());
    }
    if root.join("artisan").exists() && root.join("composer.json").exists() {
        return ("laravel".into(), "LV".into());
    }
    if root.join("composer.json").exists() {
        return ("php".into(), "PHP".into());
    }
    ("other".into(), "?".into())
}

pub fn parse_wp_config(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let cfg = expand_home(path).join("wp-config.php");
    let Ok(raw) = fs::read_to_string(&cfg) else {
        return (None, None, None);
    };
    let name = capture_define(&raw, "DB_NAME");
    let user = capture_define(&raw, "DB_USER");
    let pass = capture_define(&raw, "DB_PASSWORD");
    (name, user, pass)
}

fn capture_define(raw: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(
        r#"define\(\s*['\"]{key}['\"]\s*,\s*['\"]([^'\"]*)['\"]\s*\)"#
    ))
    .ok()?;
    re.captures(raw)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

pub fn detect_site(site: &SiteRecord) -> SiteDetails {
    let (framework, icon) = detect_framework(&site.path);
    let abs = expand_home(&site.path);
    let has_wp = abs.join("wp-config.php").exists();
    let (db_name, db_user, db_pass) = if framework == "wordpress" {
        parse_wp_config(&site.path)
    } else {
        (site.database_name.clone(), site.database_user.clone(), None)
    };

    let pma = if framework == "wordpress" {
        db_name.as_ref().map(|db| {
            let user = db_user.clone().unwrap_or_else(|| "root".into());
            let pass = db_pass.clone().unwrap_or_default();
            format!(
                "https://phpmyadmin.test/devstack-signon.php?db={}&user={}&pass={}",
                urlencoding_minimal(db),
                urlencoding_minimal(&user),
                urlencoding_minimal(&pass)
            )
        })
    } else {
        None
    };

    let mut updated = site.clone();
    updated.framework = Some(framework.clone());
    if updated.database_name.is_none() {
        updated.database_name = db_name.clone();
    }
    if updated.database_user.is_none() {
        updated.database_user = db_user.clone();
    }

    let (http_port, https_port) = if crate::services::httpd::site_uses_apache(site) {
        (
            // Ports come from stack prefs; detect_site has no AppState — use defaults
            // overridden by list_site_details which rewrites URLs when state is available.
            8088u16,
            8444u16,
        )
    } else {
        (80u16, 443u16)
    };
    let url = crate::services::ports::format_url("http", &site.domain, http_port);
    let https_url = crate::services::ports::format_url("https", &site.domain, https_port);

    SiteDetails {
        site: updated,
        framework,
        framework_icon: icon,
        absolute_path: abs,
        url,
        https_url,
        database_name: db_name,
        database_user: db_user,
        phpmyadmin_url: pma,
        has_wp_config: has_wp,
    }
}

/// Rewrite site detail URLs using live stack port preferences.
pub fn apply_stack_ports(details: &mut SiteDetails, state: &AppState) {
    let apache = crate::services::httpd::site_uses_apache(&details.site);
    let (http_port, https_port) = if apache {
        (
            state.stack.apache_http_port,
            state.stack.apache_https_port,
        )
    } else {
        (state.stack.http_port, state.stack.https_port)
    };
    details.url = crate::services::ports::format_url("http", &details.site.domain, http_port);
    details.https_url =
        crate::services::ports::format_url("https", &details.site.domain, https_port);
    if let Some(pma) = details.phpmyadmin_url.as_mut() {
        // Rebuild host so non-default Nginx HTTPS ports appear in the URL.
        if let Some(rest) = pma.strip_prefix("https://phpmyadmin.test") {
            let base = crate::services::ports::format_url(
                "https",
                "phpmyadmin.test",
                state.stack.https_port,
            );
            // rest begins with `/…` or `:port/…`
            let path = if let Some(idx) = rest.find('/') {
                &rest[idx..]
            } else {
                ""
            };
            *pma = format!("{base}{path}");
        }
    }
}

fn urlencoding_minimal(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

pub fn open_phpmyadmin_url(details: &SiteDetails) -> Option<String> {
    details.phpmyadmin_url.clone()
}

pub fn refresh_site_metadata(state: &mut AppState) {
    for site in &mut state.sites {
        site.ensure_domain();
        let details = detect_site(site);
        site.framework = Some(details.framework);
        if site.database_name.is_none() {
            site.database_name = details.database_name;
        }
        if site.database_user.is_none() {
            site.database_user = details.database_user;
        }
    }
}

pub fn create_site(
    state: &mut AppState,
    _paths: &Paths,
    name: &str,
    path: PathBuf,
    php_version: Option<String>,
) -> Result<SiteRecord> {
    let id = slugify(name);
    let mut site = SiteRecord {
        id: id.clone(),
        name: name.to_string(),
        path,
        domain: format!("{id}.test"),
        php_version,
        node_version: None,
        https: true,
        web_server: Some("nginx".into()),
        framework: None,
        database_engine: state.active_database.clone(),
        database_name: None,
        database_user: None,
        database_password: None,
        status: "stopped".into(),
        is_template: false,
    };
    let details = detect_site(&site);
    site.framework = Some(details.framework);
    site.database_name = details.database_name;
    site.database_user = details.database_user;
    // Normalize mixed separators / ~ before persisting.
    site.path = crate::paths_util::expand_home(&site.path);
    state.sites.retain(|s| s.id != site.id);
    state.sites.push(site.clone());
    let abs = crate::paths_util::ensure_dir(&site.path)?;
    site.path = abs.clone();
    if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
        s.path = abs.clone();
    }
    if !abs.join("index.php").exists() && !abs.join("index.html").exists() {
        fs::write(abs.join("index.php"), "<?php phpinfo();")?;
    }
    Ok(site)
}

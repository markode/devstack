//! Small string builders injected into generated nginx / Apache vhosts
//! (aliases, per-site logs, per-site PHP overrides).

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::config::{AppState, Paths, SiteRecord};
use crate::services::site_php_ini;

/// Reserved conf names that are not site id vhosts (keep during purge).
fn is_reserved_vhost_name(stem: &str) -> bool {
    stem == "00-default"
        || stem == "phpmyadmin"
        || stem == "mailpit"
        || stem.starts_with("mailpit")
        || stem.starts_with("mobile")
        || stem.starts_with("00-")
}

/// Delete stale site confs so a nginx site never leaves a Redirect vhost on Apache
/// (and vice versa). Keeps reserved helpers (`00-default`, phpMyAdmin, mailpit, …).
pub fn purge_stale_vhost_confs(paths: &Paths, state: &AppState) {
    let apache_ids: HashSet<String> = state
        .sites
        .iter()
        .filter(|s| crate::services::httpd::site_uses_apache(s))
        .map(|s| s.id.clone())
        .collect();
    let nginx_ids: HashSet<String> = state
        .sites
        .iter()
        .filter(|s| !crate::services::httpd::site_uses_apache(s))
        .map(|s| s.id.clone())
        .collect();

    purge_dir_keeping(&paths.apache_sites_dir(), &apache_ids);
    purge_dir_keeping(&paths.nginx_sites_dir(), &nginx_ids);
}

fn purge_dir_keeping(dir: &Path, keep_ids: &HashSet<String>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("conf") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_reserved_vhost_name(stem) {
            continue;
        }
        if !keep_ids.contains(stem) {
            let _ = fs::remove_file(&path);
        }
    }
}

/// `server_name` value: domain, aliases and `*.host` wildcards.
pub fn nginx_server_names(site: &SiteRecord) -> String {
    site.server_names().join(" ")
}

/// Per-site access/error log directives (empty-safe; creates the log dir).
pub fn nginx_log_lines(paths: &Paths, site: &SiteRecord) -> String {
    let dir = paths.logs.join("nginx");
    let _ = std::fs::create_dir_all(&dir);
    let access = dir.join(format!("{}-access.log", site.id));
    let error = dir.join(format!("{}-error.log", site.id));
    format!(
        "  access_log {};\n  error_log {} warn;",
        crate::paths_util::quoted_conf_path(&access),
        crate::paths_util::quoted_conf_path(&error),
    )
}

/// `fastcgi_param PHP_VALUE` for per-site overrides (php-fpm only; php-cgi uses `.user.ini`).
pub fn nginx_php_extra(paths: &Paths, site: &SiteRecord) -> String {
    if cfg!(windows) {
        return String::new();
    }
    let overrides = site_php_ini::load_overrides(paths, &site.id);
    match site_php_ini::php_value_param(&overrides) {
        Some(v) => format!("    fastcgi_param PHP_VALUE \"{v}\";\n"),
        None => String::new(),
    }
}

/// Apache `ServerAlias` line (with trailing newline) or empty.
pub fn apache_server_alias(site: &SiteRecord) -> String {
    let names: Vec<String> = site
        .server_names()
        .into_iter()
        .filter(|n| *n != site.domain)
        .collect();
    if names.is_empty() {
        String::new()
    } else {
        format!("  ServerAlias {}\n", names.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppState;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn site() -> SiteRecord {
        let mut s = SiteRecord {
            id: "shop".into(),
            name: "shop".into(),
            path: "/tmp/shop".into(),
            domain: "shop.test".into(),
            php_version: None,
            node_version: None,
            https: true,
            web_server: None,
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: vec!["store.test".into()],
            wildcard: true,
        };
        s.ensure_domain();
        s
    }

    #[test]
    fn purge_stale_removes_wrong_server_and_orphans() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("devstack-purge-{n}"));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths::from_root(root.clone()).unwrap();
        fs::create_dir_all(paths.apache_sites_dir()).unwrap();
        fs::create_dir_all(paths.nginx_sites_dir()).unwrap();

        // nginx site leftover in apache dir (the Windows diag failure mode)
        fs::write(paths.apache_sites_dir().join("my-site.conf"), "bad").unwrap();
        fs::write(paths.apache_sites_dir().join("00-default.conf"), "keep").unwrap();
        fs::write(paths.apache_sites_dir().join("blog.conf"), "apache").unwrap();
        fs::write(paths.nginx_sites_dir().join("my-site.conf"), "nginx").unwrap();
        fs::write(paths.nginx_sites_dir().join("blog.conf"), "stale-apache").unwrap();
        fs::write(paths.nginx_sites_dir().join("phpmyadmin.conf"), "keep").unwrap();
        fs::write(paths.nginx_sites_dir().join("gone.conf"), "orphan").unwrap();

        let mut state = AppState::default();
        state.sites.push(SiteRecord {
            id: "my-site".into(),
            name: "my-site".into(),
            path: root.join("my-site"),
            domain: "my-site.test".into(),
            php_version: Some("8.3".into()),
            node_version: None,
            https: true,
            web_server: Some("nginx".into()),
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: Vec::new(),
            wildcard: false,
        });
        state.sites.push(SiteRecord {
            id: "blog".into(),
            name: "blog".into(),
            path: root.join("blog"),
            domain: "blog.test".into(),
            php_version: Some("8.3".into()),
            node_version: None,
            https: true,
            web_server: Some("apache".into()),
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: Vec::new(),
            wildcard: false,
        });

        purge_stale_vhost_confs(&paths, &state);

        assert!(!paths.apache_sites_dir().join("my-site.conf").exists());
        assert!(paths.apache_sites_dir().join("blog.conf").exists());
        assert!(paths.apache_sites_dir().join("00-default.conf").exists());
        assert!(paths.nginx_sites_dir().join("my-site.conf").exists());
        assert!(!paths.nginx_sites_dir().join("blog.conf").exists());
        assert!(paths.nginx_sites_dir().join("phpmyadmin.conf").exists());
        assert!(!paths.nginx_sites_dir().join("gone.conf").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn server_names_include_aliases_and_wildcards() {
        let n = nginx_server_names(&site());
        assert_eq!(n, "shop.test store.test *.shop.test *.store.test");
    }

    #[test]
    fn apache_alias_line() {
        assert_eq!(
            apache_server_alias(&site()),
            "  ServerAlias store.test *.shop.test *.store.test\n"
        );
        let mut s = site();
        s.aliases.clear();
        s.wildcard = false;
        assert_eq!(apache_server_alias(&s), "");
    }
}

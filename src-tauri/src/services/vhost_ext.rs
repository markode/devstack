//! Small string builders injected into generated nginx / Apache vhosts
//! (aliases, per-site logs, per-site PHP overrides).

use crate::config::{Paths, SiteRecord};
use crate::services::site_php_ini;

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

//! Custom TLDs, site aliases and wildcard options.

use serde::Serialize;

use crate::config::{slugify, AppState, SiteRecord};
use crate::error::{DevStackError, Result};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TldSettings {
    pub default_tld: String,
    pub warnings: Vec<String>,
    /// True for TLDs browsers force onto HTTPS (`.dev`, `.app`, …).
    pub https_required: bool,
}

const HSTS_TLDS: &[&str] = &["dev", "app", "page", "new", "day"];
const RESERVED_REAL_TLDS: &[&str] = &["com", "net", "org", "io", "co", "edu", "gov"];

/// Lowercase, strip a leading dot, validate label syntax.
pub fn normalize_tld(raw: &str) -> Result<String> {
    let t = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    if t.is_empty() {
        return Err(DevStackError::Message("TLD cannot be empty".into()));
    }
    if t.len() > 24
        || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        || t.starts_with('-')
        || t.ends_with('-')
    {
        return Err(DevStackError::Message(format!("invalid TLD: .{t}")));
    }
    Ok(t)
}

pub fn tld_warnings(tld: &str) -> Vec<String> {
    let mut w = Vec::new();
    if HSTS_TLDS.contains(&tld) {
        w.push(format!(
            ".{tld} is HSTS-preloaded: browsers only open it over HTTPS, so HTTPS is forced for these sites and the DevStack CA must be trusted."
        ));
    }
    if tld == "local" {
        w.push(
            ".local is reserved for mDNS (Bonjour/Avahi): lookups can be slow or bypass the hosts file on macOS and Linux. Prefer .test.".into(),
        );
    }
    if tld == "localhost" {
        w.push(".localhost resolves to loopback in most browsers but cannot be shared across devices.".into());
    }
    if RESERVED_REAL_TLDS.contains(&tld) {
        w.push(format!(
            ".{tld} is a real public TLD: the hosts file will shadow the live site on this machine."
        ));
    }
    if tld != "test" {
        w.push(
            "Wildcard sub-domains on non-.test TLDs rely on the hosts file only (list aliases explicitly) on macOS.".into(),
        );
    }
    w
}

pub fn tld_settings(tld: &str) -> TldSettings {
    TldSettings {
        default_tld: tld.to_string(),
        warnings: tld_warnings(tld),
        https_required: HSTS_TLDS.contains(&tld),
    }
}

pub fn set_default_tld(state: &mut AppState, raw: &str) -> Result<TldSettings> {
    let tld = normalize_tld(raw)?;
    state.developer.default_tld = tld.clone();
    Ok(tld_settings(&tld))
}

/// Validate an alias hostname (`shop.test`, `api.shop.local`).
pub fn normalize_alias(raw: &str) -> Result<String> {
    let a = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if a.is_empty() {
        return Err(DevStackError::Message("alias cannot be empty".into()));
    }
    if a.len() > 253 || !a.contains('.') {
        return Err(DevStackError::Message(format!(
            "alias must be a full hostname like shop.test (got {raw})"
        )));
    }
    for label in a.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(DevStackError::Message(format!("invalid alias hostname: {raw}")));
        }
    }
    Ok(a)
}

/// Replace the last label of `domain` with `tld` (`blog.test` → `blog.dev`).
pub fn domain_with_tld(domain: &str, tld: &str) -> String {
    match domain.rsplit_once('.') {
        Some((head, _)) => format!("{head}.{tld}"),
        None => format!("{domain}.{tld}"),
    }
}

/// Domain for a brand-new site name under `tld`.
pub fn domain_for_name(name: &str, tld: &str) -> String {
    format!("{}.{}", slugify(name), tld)
}

fn taken_elsewhere(state: &AppState, site_id: &str, host: &str) -> bool {
    state
        .sites
        .iter()
        .filter(|s| s.id != site_id)
        .any(|s| s.hostnames().iter().any(|h| h == host))
}

/// Apply TLD / aliases / wildcard changes onto one site. Caller syncs hosts + vhosts.
pub fn update_site_domain_options(
    state: &mut AppState,
    site_id: &str,
    tld: Option<&str>,
    aliases: Option<Vec<String>>,
    wildcard: Option<bool>,
) -> Result<(SiteRecord, Vec<String>)> {
    let mut warnings = Vec::new();
    let new_tld = tld.map(normalize_tld).transpose()?;
    let new_aliases = match aliases {
        Some(list) => {
            let mut out: Vec<String> = Vec::new();
            for a in list {
                if a.trim().is_empty() {
                    continue;
                }
                let n = normalize_alias(&a)?;
                if !out.contains(&n) {
                    out.push(n);
                }
            }
            Some(out)
        }
        None => None,
    };

    let idx = state
        .sites
        .iter()
        .position(|s| s.id == site_id)
        .ok_or_else(|| DevStackError::Message(format!("site not found: {site_id}")))?;

    if let Some(tld) = &new_tld {
        let domain = domain_with_tld(&state.sites[idx].domain, tld);
        if taken_elsewhere(state, site_id, &domain) {
            return Err(DevStackError::Message(format!("{domain} is already used by another site")));
        }
        warnings.extend(tld_warnings(tld));
        let site = &mut state.sites[idx];
        site.domain = domain;
        site.tld = tld.clone();
        if HSTS_TLDS.contains(&tld.as_str()) {
            site.https = true;
        }
    }
    if let Some(list) = new_aliases {
        for a in &list {
            if taken_elsewhere(state, site_id, a) {
                return Err(DevStackError::Message(format!("{a} is already used by another site")));
            }
            if let Some((_, t)) = a.rsplit_once('.') {
                warnings.extend(tld_warnings(t));
            }
        }
        let primary = state.sites[idx].domain.clone();
        state.sites[idx].aliases = list.into_iter().filter(|a| *a != primary).collect();
    }
    if let Some(w) = wildcard {
        state.sites[idx].wildcard = w;
    }
    warnings.sort();
    warnings.dedup();
    Ok((state.sites[idx].clone(), warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(ids: &[(&str, &str)]) -> AppState {
        let mut st = AppState::default();
        for (id, dom) in ids {
            let mut s = SiteRecord {
                id: (*id).into(),
                name: (*id).into(),
                path: "/tmp".into(),
                domain: (*dom).into(),
                php_version: None,
                node_version: None,
                https: false,
                web_server: None,
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
            st.sites.push(s);
        }
        st
    }

    #[test]
    fn normalizes_and_rejects_tlds() {
        assert_eq!(normalize_tld(".Test").unwrap(), "test");
        assert!(normalize_tld("").is_err());
        assert!(normalize_tld("a b").is_err());
        assert!(normalize_tld("-x").is_err());
    }

    #[test]
    fn dev_and_local_warn() {
        assert!(tld_settings("dev").https_required);
        assert!(tld_warnings("dev").iter().any(|w| w.contains("HTTPS")));
        assert!(tld_warnings("local").iter().any(|w| w.contains("mDNS")));
        assert!(tld_warnings("test").is_empty());
    }

    #[test]
    fn alias_validation() {
        assert_eq!(normalize_alias("Shop.Test").unwrap(), "shop.test");
        assert!(normalize_alias("nodots").is_err());
        assert!(normalize_alias("bad_label.test").is_err());
    }

    #[test]
    fn changing_tld_rewrites_domain_and_forces_https_for_dev() {
        let mut st = state_with(&[("blog", "blog.test")]);
        let (site, warnings) = update_site_domain_options(&mut st, "blog", Some("dev"), None, None).unwrap();
        assert_eq!(site.domain, "blog.dev");
        assert!(site.https);
        assert!(warnings.iter().any(|w| w.contains("HSTS")));
    }

    #[test]
    fn aliases_wildcard_and_conflicts() {
        let mut st = state_with(&[("blog", "blog.test"), ("shop", "shop.test")]);
        let (site, _) = update_site_domain_options(
            &mut st,
            "blog",
            None,
            Some(vec!["Journal.test".into(), "journal.test".into(), "blog.test".into()]),
            Some(true),
        )
        .unwrap();
        assert_eq!(site.aliases, vec!["journal.test"]);
        assert!(site.wildcard);
        assert!(update_site_domain_options(&mut st, "blog", None, Some(vec!["shop.test".into()]), None).is_err());
        assert!(update_site_domain_options(&mut st, "blog", Some("test"), None, None).is_ok());
    }

    #[test]
    fn default_tld_setting() {
        let mut st = AppState::default();
        let s = set_default_tld(&mut st, ".local").unwrap();
        assert_eq!(st.developer.default_tld, "local");
        assert!(!s.warnings.is_empty());
        assert_eq!(domain_for_name("My Blog", "local"), "my-blog.local");
    }
}

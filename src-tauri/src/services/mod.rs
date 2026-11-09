//! Runtime services: Nginx, PHP-FPM/CGI, databases, DNS, TLS, site detection.

pub mod backup;
pub mod database;
pub mod db_files;
pub mod dns;
pub mod dump;
pub mod extras;
pub mod httpd;
pub mod nginx;
pub mod parked;
pub mod php_runtime;
pub mod pma_signon;
pub mod ports;
pub mod process;
pub mod progress;
pub mod share;
pub mod site_backup;
pub mod sites;
pub mod stack;
pub mod templates;
pub mod tls;
pub mod tools;
pub mod wordpress;
pub mod wp_url;
pub mod xdebug_site;

pub use stack::{apply_stack, stack_status, start_stack, stop_stack, StackStatus};
pub use sites::{detect_site, open_phpmyadmin_url, SiteDetails};

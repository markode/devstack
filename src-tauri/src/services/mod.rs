//! Runtime services: Nginx, PHP-FPM/CGI, databases, DNS, TLS, site detection.

pub mod database;
pub mod dns;
pub mod nginx;
pub mod php_runtime;
pub mod process;
pub mod sites;
pub mod stack;
pub mod tls;

pub use stack::{apply_stack, stack_status, start_stack, stop_stack, StackStatus};
pub use sites::{detect_site, open_phpmyadmin_url, SiteDetails};

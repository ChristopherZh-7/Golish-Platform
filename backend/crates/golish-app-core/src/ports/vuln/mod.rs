//! Vuln service outbound ports (servitization S1-2c).
//!
//! The consuming agent service holds `Arc<dyn VulnIntelPort>` / `Arc<dyn
//! WikiKbPort>` instead of calling `golish_db::repo::{vuln_intel,wiki_kb}`
//! directly. Each `Pg*Adapter` is the single guarded repo-calling site.

pub mod intel;
pub mod wiki;

pub use intel::{PgVulnIntelAdapter, VulnIntelPort};
pub use wiki::{PgWikiKbAdapter, WikiKbPort};

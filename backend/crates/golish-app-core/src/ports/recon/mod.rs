//! Recon service outbound ports (servitization S1-2b).
//!
//! Splits recon's cross-service repo surface into per-group sub-ports (design
//! decision Y): the consuming agent / pentest / platform / vuln services hold
//! `Arc<dyn Recon*Port>` instead of calling `golish_db::repo::<recon table>`
//! directly. Each `Pg*Adapter` is the single guarded repo-calling site.
//!
//! b1 introduced `scans` (api_endpoints / js_analysis / fingerprints /
//! passive_scans) + `assets` (target_assets); b2 extended `scans`; b3/b4/b6 add
//! `targets` / `sitemap` / `directory`.

pub mod assets;
pub mod directory;
pub mod scans;
pub mod sitemap;
pub mod targets;

pub use assets::{PgReconAssetsAdapter, ReconAssetsPort};
pub use directory::{ConditionalDirectoryEntryWrite, PgReconDirectoryAdapter, ReconDirectoryPort};
pub use scans::{PgReconScansAdapter, ReconPassiveScanGlobal, ReconScansPort};
pub use sitemap::{PgReconSitemapAdapter, ReconSitemapPort};
pub use targets::{PgReconTargetsAdapter, ReconTargetsPort};

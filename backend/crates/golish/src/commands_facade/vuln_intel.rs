//! Vulnerability intelligence commands.

pub use crate::tools::vuln_intel::{
    intel_list_feeds, intel_add_feed, intel_toggle_feed, intel_delete_feed,
    intel_fetch, intel_fetch_page, intel_get_cached,
    intel_search, intel_search_remote, intel_search_remote_page,
    intel_match_targets, intel_search_github_poc,
    intel_search_nuclei_templates, intel_batch_search_nuclei_templates,
    intel_discover_all_nuclei,
};

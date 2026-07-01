//! Provider-side service ports (servitization S1-2).
//!
//! Each submodule holds one service's outbound port: a `*Port` trait (the
//! remote-ready contract — only serializable params, no `PgPool`/closures) and
//! an in-proc adapter (`Pg*Adapter`) that is the ONLY place allowed to call
//! `golish_db::repo::<that service>`. Consumers hold `Arc<dyn *Port>` and never
//! touch another service's repo directly. See
//! `docs/design/2026-05-30-s1-2-port-horizontal-coupling.md`.

pub mod agent;
pub mod llm;
pub mod pentest;
pub mod platform;
pub mod recon;
pub mod vuln;

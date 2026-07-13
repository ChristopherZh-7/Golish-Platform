fn main() {
    // `sqlx::migrate!` embeds the migration directory at compile time. Without
    // this build-script dependency Cargo can reuse a stale test binary after a
    // new migration file is added.
    println!("cargo:rerun-if-changed=migrations");
}

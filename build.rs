fn main() {
    // `embed_migrations!` cannot tell Cargo to rebuild when a migration is added or edited.
    println!("cargo::rerun-if-changed=migrations");
}

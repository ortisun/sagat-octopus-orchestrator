//! `sqlx::migrate!` embeds the migration files at compile time; without this, editing or adding a
//! `.sql` file would not rebuild the crate and the old schema would keep being embedded.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}

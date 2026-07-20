//! Temporary diagnostic for the 4K Stogram migration: prints what the gallery
//! actually returns for a profile (sections, albums, tiny files), which is the
//! only reliable way to see what the ProfileView sees.
//!
//! Delete together with `stogram_migration_cli` once the migration is done.
//!
//! ```text
//! cargo run --release -p ninjacrawler --bin stogram_diag_cli -- carolbaudien
//! ```

use std::collections::BTreeMap;
use std::env;
use std::process;

use ninjacrawler_lib::infrastructure::workspace_repository::{
    bootstrap_workspace, load_source_media_gallery,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("stogram_diag_cli failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let handles = env::args().skip(1).collect::<Vec<_>>();
    if handles.is_empty() {
        return Err("pass one or more handles".to_string());
    }

    let snapshot = bootstrap_workspace()?;
    for handle in handles {
        let Some(source) = snapshot.sources.iter().find(|source| {
            source.provider.eq_ignore_ascii_case("instagram")
                && source.handle.trim_start_matches('@').eq_ignore_ascii_case(&handle)
        }) else {
            println!("== {handle}: NOT FOUND");
            continue;
        };

        let gallery = load_source_media_gallery(source.id.clone())?;
        println!("== {} ({} posts)", handle, gallery.posts.len());

        let mut by_section: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_album: BTreeMap<String, usize> = BTreeMap::new();
        let mut no_album_in_stories = 0usize;
        let mut tiny_files = 0usize;
        for post in &gallery.posts {
            *by_section.entry(post.section.clone()).or_default() += 1;
            let albums = post.albums.clone();
            if albums.is_empty() {
                if post.section == "stories" {
                    no_album_in_stories += 1;
                }
            } else {
                for album in albums {
                    *by_album.entry(album).or_default() += 1;
                }
            }
            for file in &post.files {
                if std::fs::metadata(&file.absolute_path)
                    .map(|meta| meta.len() < 20 * 1024)
                    .unwrap_or(false)
                {
                    tiny_files += 1;
                }
            }
        }

        println!("   sections: {by_section:?}");
        println!("   albums:   {by_album:?}");
        println!("   stories posts WITHOUT album: {no_album_in_stories}");
        println!("   files under 20 KB (thumbnail-sized): {tiny_files}");
    }
    Ok(())
}

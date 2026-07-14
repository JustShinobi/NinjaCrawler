use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn is_nonempty_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

pub(crate) fn write_bytes_replacing_empty(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("empty response body".to_string());
    }

    let temporary = temporary_path(destination)?;
    if let Err(error) = fs::write(&temporary, bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    if fs::metadata(&temporary)
        .map(|metadata| metadata.len() != bytes.len() as u64)
        .unwrap_or(true)
    {
        let _ = fs::remove_file(&temporary);
        return Err("temporary download was not written completely".to_string());
    }

    commit_temporary_replacing_empty(&temporary, destination)
}

pub(crate) fn copy_file_replacing_empty(source: &Path, destination: &Path) -> Result<(), String> {
    let source_size = fs::metadata(source)
        .map_err(|error| error.to_string())?
        .len();
    if source_size == 0 {
        return Err("downloaded source file is empty".to_string());
    }

    let temporary = temporary_path(destination)?;
    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    if fs::metadata(&temporary)
        .map(|metadata| metadata.len() != source_size)
        .unwrap_or(true)
    {
        let _ = fs::remove_file(&temporary);
        return Err("temporary copy was not written completely".to_string());
    }

    commit_temporary_replacing_empty(&temporary, destination)
}

fn temporary_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "download destination has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let final_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "download destination has an invalid file name".to_string())?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{final_name}.{}.{}.part",
        std::process::id(),
        sequence
    )))
}

fn commit_temporary_replacing_empty(temporary: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        if is_nonempty_file(destination) {
            let _ = fs::remove_file(temporary);
            return Err("destination file already exists".to_string());
        }
        if let Err(error) = fs::remove_file(destination) {
            let _ = fs::remove_file(temporary);
            return Err(error.to_string());
        }
    }

    if let Err(error) = fs::rename(temporary, destination) {
        let _ = fs::remove_file(temporary);
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_empty_destination_without_part_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("media.mp4");
        fs::write(&destination, []).expect("placeholder");

        write_bytes_replacing_empty(&destination, b"complete media").expect("atomic write");

        assert_eq!(fs::read(&destination).expect("download"), b"complete media");
        assert_no_part_files(temp.path());
    }

    #[test]
    fn atomic_write_preserves_nonempty_destination() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("media.mp4");
        fs::write(&destination, b"existing media").expect("existing");

        let error = write_bytes_replacing_empty(&destination, b"replacement").expect_err("blocked");

        assert_eq!(error, "destination file already exists");
        assert_eq!(fs::read(&destination).expect("existing"), b"existing media");
        assert_no_part_files(temp.path());
    }

    #[test]
    fn atomic_copy_rejects_empty_source_without_creating_destination() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.mp4");
        let destination = temp.path().join("destination.mp4");
        fs::write(&source, []).expect("placeholder");

        let error = copy_file_replacing_empty(&source, &destination).expect_err("empty source");

        assert_eq!(error, "downloaded source file is empty");
        assert!(!destination.exists());
        assert_no_part_files(temp.path());
    }

    #[test]
    fn atomic_copy_replaces_empty_destination() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.mp4");
        let destination = temp.path().join("destination.mp4");
        fs::write(&source, b"complete media").expect("source");
        fs::write(&destination, []).expect("placeholder");

        copy_file_replacing_empty(&source, &destination).expect("atomic copy");

        assert_eq!(
            fs::read(&destination).expect("destination"),
            b"complete media"
        );
        assert_eq!(fs::read(&source).expect("source"), b"complete media");
        assert_no_part_files(temp.path());
    }

    #[test]
    fn atomic_write_rejects_empty_payload_without_creating_destination() {
        let temp = tempfile::tempdir().expect("tempdir");
        let destination = temp.path().join("media.mp4");

        let error = write_bytes_replacing_empty(&destination, &[]).expect_err("empty payload");

        assert_eq!(error, "empty response body");
        assert!(!destination.exists());
        assert_no_part_files(temp.path());
    }

    fn assert_no_part_files(directory: &Path) {
        assert!(fs::read_dir(directory).expect("entries").all(|entry| !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .ends_with(".part")));
    }
}

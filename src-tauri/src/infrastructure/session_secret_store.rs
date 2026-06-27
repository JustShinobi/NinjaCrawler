use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::slice;

use crate::infrastructure::storage::StorageLayout;

const SESSION_SECRET_EXTENSION: &str = "bin";
const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

#[repr(C)]
struct DataBlob {
    cb_data: u32,
    pb_data: *mut u8,
}

#[cfg(target_os = "windows")]
#[link(name = "Crypt32")]
unsafe extern "system" {
    fn CryptProtectData(
        p_data_in: *const DataBlob,
        sz_data_descr: *const u16,
        p_optional_entropy: *const DataBlob,
        pv_reserved: *mut c_void,
        p_prompt_struct: *mut c_void,
        dw_flags: u32,
        p_data_out: *mut DataBlob,
    ) -> i32;

    fn CryptUnprotectData(
        p_data_in: *const DataBlob,
        ppsz_data_descr: *mut *mut u16,
        p_optional_entropy: *const DataBlob,
        pv_reserved: *mut c_void,
        p_prompt_struct: *mut c_void,
        dw_flags: u32,
        p_data_out: *mut DataBlob,
    ) -> i32;
}

#[cfg(target_os = "windows")]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
}

pub fn store_secret(layout: &StorageLayout, secret_ref: &str, payload: &str) -> Result<(), String> {
    let path = secret_path(layout, secret_ref)?;
    let ciphertext = protect_bytes(payload.as_bytes())?;
    fs::write(path, ciphertext).map_err(|error| error.to_string())
}

pub fn load_secret(layout: &StorageLayout, secret_ref: &str) -> Result<String, String> {
    let path = secret_path(layout, secret_ref)?;
    let ciphertext = fs::read(path).map_err(|error| error.to_string())?;
    let plaintext = unprotect_bytes(&ciphertext)?;
    String::from_utf8(plaintext).map_err(|error| error.to_string())
}

pub fn delete_secret(layout: &StorageLayout, secret_ref: &str) -> Result<(), String> {
    let path = secret_path(layout, secret_ref)?;

    if !path.exists() {
        return Ok(());
    }

    fs::remove_file(path).map_err(|error| error.to_string())
}

pub fn has_secret(layout: &StorageLayout, secret_ref: &str) -> Result<bool, String> {
    Ok(secret_path(layout, secret_ref)?.exists())
}

fn secret_path(layout: &StorageLayout, secret_ref: &str) -> Result<PathBuf, String> {
    let safe_ref = secret_ref.trim();
    if safe_ref.is_empty() {
        return Err("Session secret reference cannot be empty.".to_string());
    }

    let root = session_secret_root(layout)?;
    Ok(root.join(format!("{safe_ref}.{SESSION_SECRET_EXTENSION}")))
}

fn session_secret_root(layout: &StorageLayout) -> Result<PathBuf, String> {
    let root = layout.data_dir.join("sessions");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

fn protect_bytes(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        let input = DataBlob {
            cb_data: plaintext.len() as u32,
            pb_data: plaintext.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: null_mut(),
        };

        let result = CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            null_mut(),
            null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        );

        if result == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }

        let bytes = owned_blob_bytes(&output);
        free_blob(&mut output);
        bytes
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = plaintext;
        Err("Session secret protection is only implemented for Windows.".to_string())
    }
}

fn unprotect_bytes(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        let input = DataBlob {
            cb_data: ciphertext.len() as u32,
            pb_data: ciphertext.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: null_mut(),
        };

        let result = CryptUnprotectData(
            &input,
            null_mut(),
            std::ptr::null(),
            null_mut(),
            null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        );

        if result == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }

        let bytes = owned_blob_bytes(&output);
        free_blob(&mut output);
        bytes
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = ciphertext;
        Err("Session secret protection is only implemented for Windows.".to_string())
    }
}

#[cfg(target_os = "windows")]
unsafe fn owned_blob_bytes(blob: &DataBlob) -> Result<Vec<u8>, String> {
    if blob.pb_data.is_null() {
        return Ok(Vec::new());
    }

    Ok(slice::from_raw_parts(blob.pb_data, blob.cb_data as usize).to_vec())
}

#[cfg(target_os = "windows")]
unsafe fn free_blob(blob: &mut DataBlob) {
    if !blob.pb_data.is_null() {
        let _ = LocalFree(blob.pb_data as *mut c_void);
        blob.pb_data = null_mut();
        blob.cb_data = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::storage;
    use tempfile::TempDir;

    fn create_test_layout() -> (TempDir, StorageLayout) {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let local_app_data = temp_dir.path().join("localappdata");
        let user_profile = temp_dir.path().join("userprofile");
        let layout =
            storage::workspace_layout_from_roots(local_app_data, user_profile).expect("layout");
        (temp_dir, layout)
    }

    #[test]
    fn session_secret_round_trips_through_protected_store() {
        let (_temp_dir, layout) = create_test_layout();

        store_secret(&layout, "account-1", "{\"cookie\":\"value\"}").expect("store secret");
        assert!(has_secret(&layout, "account-1").expect("secret presence"));

        let restored = load_secret(&layout, "account-1").expect("load secret");
        assert_eq!(restored, "{\"cookie\":\"value\"}");

        delete_secret(&layout, "account-1").expect("delete secret");
        assert!(!has_secret(&layout, "account-1").expect("secret absence"));
    }
}

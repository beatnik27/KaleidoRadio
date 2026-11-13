use crate::model::RemoteConnection;

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{CStr, CString, c_char};
    unsafe extern "C" {
        fn kr_password_load(account: *const c_char) -> *mut c_char;
        fn kr_password_free(password: *mut c_char);
        fn kr_password_save(account: *const c_char, password: *const c_char) -> i32;
        fn kr_password_delete(account: *const c_char) -> i32;
    }
    pub fn load(account: &str) -> Option<String> {
        let account = CString::new(account).ok()?;
        unsafe {
            let password = kr_password_load(account.as_ptr());
            if password.is_null() {
                return None;
            }
            let value = CStr::from_ptr(password).to_string_lossy().into_owned();
            kr_password_free(password);
            Some(value)
        }
    }
    pub fn save(account: &str, password: &str) -> Result<(), String> {
        let account = CString::new(account).map_err(|_| "Invalid server account")?;
        let password = CString::new(password).map_err(|_| "Password contains a null character")?;
        let status = unsafe { kr_password_save(account.as_ptr(), password.as_ptr()) };
        if status == 0 {
            Ok(())
        } else {
            Err(format!("Could not save password in Keychain ({status})."))
        }
    }
    pub fn delete(account: &str) -> Result<(), String> {
        let account = CString::new(account).map_err(|_| "Invalid server account")?;
        let status = unsafe { kr_password_delete(account.as_ptr()) };
        if status == 0 {
            Ok(())
        } else {
            Err(format!(
                "Could not remove password from Keychain ({status})."
            ))
        }
    }
}
#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn load(_: &str) -> Option<String> {
        None
    }
    pub fn save(_: &str, _: &str) -> Result<(), String> {
        Err("Password storage is unavailable on this platform.".into())
    }
    pub fn delete(_: &str) -> Result<(), String> {
        Ok(())
    }
}
fn account(connection: &RemoteConnection) -> String {
    serde_json::to_string(&(&connection.url, &connection.username)).unwrap()
}
pub fn load(connection: &RemoteConnection) -> String {
    platform::load(&account(connection)).unwrap_or_default()
}
pub fn save(connection: &RemoteConnection, password: &str) -> Result<(), String> {
    platform::save(&account(connection), password)
}
pub fn delete(connection: &RemoteConnection) -> Result<(), String> {
    platform::delete(&account(connection))
}

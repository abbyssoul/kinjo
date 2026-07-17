//! Shared XDG config-home resolution for command rules and keybindings.

use std::{ffi::OsString, path::PathBuf};

/// Return Kinjo's directory under one valid config home.
///
/// The XDG Base Directory Specification treats an empty variable as unset and
/// requires referenced paths to be absolute. `HOME` follows the same safety
/// rule here: resolving trusted configuration relative to the process working
/// directory would make launch location silently change what Kinjo loads.
pub(crate) fn kinjo_config_dir(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    valid_absolute(xdg_config_home)
        .or_else(|| valid_absolute(home).map(|home| home.join(".config")))
        .map(|base| base.join("kinjo"))
}

fn valid_absolute(value: Option<OsString>) -> Option<PathBuf> {
    let path = PathBuf::from(value?);
    (!path.as_os_str().is_empty() && path.is_absolute()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::absolute;

    #[test]
    fn xdg_wins_and_home_is_the_fallback() {
        assert_eq!(
            kinjo_config_dir(
                Some(absolute("/xdg").into_os_string()),
                Some(absolute("/home/user").into_os_string())
            ),
            Some(absolute("/xdg/kinjo"))
        );
        assert_eq!(
            kinjo_config_dir(None, Some(absolute("/home/user").into_os_string())),
            Some(absolute("/home/user/.config/kinjo"))
        );
    }

    #[test]
    fn empty_and_relative_values_are_ignored() {
        assert_eq!(
            kinjo_config_dir(
                Some(OsString::new()),
                Some(absolute("/home/user").into_os_string())
            ),
            Some(absolute("/home/user/.config/kinjo"))
        );
        assert_eq!(
            kinjo_config_dir(
                Some(OsString::from("relative")),
                Some(OsString::from("also-relative"))
            ),
            None
        );
    }
}

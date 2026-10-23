//! Current and legacy preference locations share the same platform base.
use std::path::PathBuf;

pub(crate) fn paths(filename: &str) -> Option<(PathBuf, PathBuf)> {
    #[cfg(target_os = "windows")]
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let appdata = None;
    let base = config_base(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        appdata,
        std::env::var_os("HOME").map(PathBuf::from),
    )?;
    Some((
        base.join("rdnmb").join(filename),
        base.join("nmbxd").join(filename),
    ))
}

fn config_base(
    xdg: Option<PathBuf>,
    appdata: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    xdg.filter(|path| path.is_absolute())
        .or_else(|| appdata.filter(|path| path.is_absolute()))
        .or_else(|| {
            home.filter(|path| !path.as_os_str().is_empty())
                .map(|path| path.join(".config"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_base_precedence_ignores_empty_and_relative_overrides() {
        let base = std::env::temp_dir();
        let xdg = base.join("xdg");
        let appdata = base.join("appdata");
        let home = base.join("home");
        assert_eq!(
            config_base(Some(xdg.clone()), Some(appdata.clone()), Some(home.clone())),
            Some(xdg)
        );
        assert_eq!(
            config_base(
                Some("relative".into()),
                Some(appdata.clone()),
                Some(home.clone())
            ),
            Some(appdata)
        );
        assert_eq!(
            config_base(
                Some(PathBuf::new()),
                Some("relative".into()),
                Some(home.clone())
            ),
            Some(home.join(".config"))
        );
        assert_eq!(config_base(None, None, Some(PathBuf::new())), None);
    }
}

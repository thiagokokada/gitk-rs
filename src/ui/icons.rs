use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

pub(super) const APP_ICON: &str = "icons/gitk-rs.svg";

pub(super) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            Ok(Some(Cow::Borrowed(include_bytes!(
                "../../resources/icons/gitk-rs.svg"
            ))))
        } else {
            gpui_component_assets::Assets.load(path)
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        if APP_ICON.starts_with(path) {
            assets.push(APP_ICON.into());
        }
        Ok(assets)
    }
}

//! Resolve system font choices before the application starts rendering.
#[cfg(all(target_os = "linux", feature = "system-fonts"))]
use anyhow::{Context, ensure};
use gitk_rs::preferences::{SYSTEM_MONOSPACE_FONT, SYSTEM_UI_FONT};
use gpui_kit::{App, Global, SharedString};

pub(super) struct SystemFonts {
    ui: SharedString,
    monospace: SharedString,
}

impl Global for SystemFonts {}

impl SystemFonts {
    pub(super) fn load() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::from_matches(|generic| {
                #[cfg(feature = "system-fonts")]
                {
                    fontconfig_family(generic).ok()
                }
                #[cfg(not(feature = "system-fonts"))]
                {
                    let _ = generic;
                    None
                }
            })
        }
        #[cfg(target_os = "macos")]
        {
            Self {
                ui: ".SystemUIFont".into(),
                monospace: crate::macos::system_monospace_font().into(),
            }
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn from_matches(mut find: impl FnMut(&str) -> Option<String>) -> Self {
        Self {
            ui: find("sans-serif")
                .unwrap_or_else(|| "DejaVu Sans".into())
                .into(),
            monospace: find("monospace")
                .unwrap_or_else(|| "DejaVu Sans Mono".into())
                .into(),
        }
    }

    fn resolve(&self, family: &str) -> SharedString {
        match family {
            SYSTEM_UI_FONT => self.ui.clone(),
            SYSTEM_MONOSPACE_FONT => self.monospace.clone(),
            _ => family.to_owned().into(),
        }
    }
}

pub(super) fn resolve(family: &str, cx: &App) -> SharedString {
    // Headless GPUI tests accept font names without consulting the host system.
    cx.try_global::<SystemFonts>()
        .map_or_else(|| family.to_owned().into(), |fonts| fonts.resolve(family))
}

#[cfg(all(target_os = "linux", feature = "system-fonts"))]
fn fontconfig_family(generic: &str) -> anyhow::Result<String> {
    let fc = fontconfig::Fontconfig::new().context("initialize Fontconfig")?;
    let mut pattern = fontconfig::Pattern::new(&fc)?;
    pattern.add_string(fontconfig::FC_FAMILY, &std::ffi::CString::new(generic)?)?;
    let matched = pattern.font_match()?;
    let family = matched.get_string(fontconfig::FC_FAMILY)?.to_owned();
    ensure!(
        !family.trim().is_empty(),
        "no system font found for {generic}"
    );
    Ok(family)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_os = "linux", not(feature = "system-fonts")))]
    #[test]
    fn disabled_resolution_loads_hardcoded_defaults() {
        let fonts = SystemFonts::load();
        assert_eq!(fonts.resolve(SYSTEM_UI_FONT).as_ref(), "DejaVu Sans");
        assert_eq!(
            fonts.resolve(SYSTEM_MONOSPACE_FONT).as_ref(),
            "DejaVu Sans Mono"
        );
    }

    #[test]
    fn unavailable_fontconfig_and_partial_matches_use_hardcoded_defaults() {
        let fonts = SystemFonts::from_matches(|_| None);
        assert_eq!(fonts.resolve(SYSTEM_UI_FONT).as_ref(), "DejaVu Sans");
        assert_eq!(
            fonts.resolve(SYSTEM_MONOSPACE_FONT).as_ref(),
            "DejaVu Sans Mono"
        );
        let fonts = SystemFonts::from_matches(|generic| {
            (generic == "sans-serif").then(|| "Desktop Sans".into())
        });
        assert_eq!(fonts.resolve(SYSTEM_UI_FONT).as_ref(), "Desktop Sans");
        assert_eq!(
            fonts.resolve(SYSTEM_MONOSPACE_FONT).as_ref(),
            "DejaVu Sans Mono"
        );
    }

    #[gpui_kit::test]
    fn applies_system_families_without_saving_the_resolved_names(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.toml");
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(SystemFonts {
                ui: "Desktop Sans".into(),
                monospace: "Desktop Mono".into(),
            });
            let preferences = gitk_rs::preferences::Preferences::default();
            super::super::apply_fonts(&preferences, cx);
            let theme = gpui_kit::component::Theme::global(cx);
            assert_eq!(theme.font_family.as_ref(), "Desktop Sans");
            assert_eq!(theme.mono_font_family.as_ref(), "Desktop Mono");
            preferences.save_to(&path).unwrap();
            let saved = gitk_rs::preferences::Preferences::load_from(&path).unwrap();
            assert_eq!(saved.ui_font, SYSTEM_UI_FONT);
            assert_eq!(saved.fixed_font, SYSTEM_MONOSPACE_FONT);
        });
    }

    #[test]
    fn resolves_system_choices_and_preserves_explicit_families() {
        let fonts = SystemFonts {
            ui: "Desktop Sans".into(),
            monospace: "Desktop Mono".into(),
        };
        assert_eq!(fonts.resolve(SYSTEM_UI_FONT).as_ref(), "Desktop Sans");
        assert_eq!(
            fonts.resolve(SYSTEM_MONOSPACE_FONT).as_ref(),
            "Desktop Mono"
        );
        assert_eq!(fonts.resolve("Custom Font").as_ref(), "Custom Font");
    }
}

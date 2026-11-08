//! Native application integration for standalone macOS launches.

use anyhow::{Context, Result};
use objc2::{AnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSApplication, NSFont, NSImage};
use objc2_foundation::NSData;

pub fn system_monospace_font() -> String {
    NSFont::monospacedSystemFontOfSize_weight(0., 0.)
        .fontName()
        .to_string()
}

fn application_icon() -> Result<Retained<NSImage>> {
    // Embed the rasterized SVG so launch directory and installed resources do
    // not affect the Dock or Cmd+Tab icon.
    let data = NSData::with_bytes(include_bytes!("../resources/icons/gitk-rs.png"));
    NSImage::initWithData(NSImage::alloc(), &data).context("invalid embedded application icon")
}

pub fn set_application_icon() -> Result<()> {
    let main_thread =
        MainThreadMarker::new().context("application icon requires the main thread")?;
    let image = application_icon()?;
    set_icon(&NSApplication::sharedApplication(main_thread), &image);
    Ok(())
}

#[allow(unsafe_code)]
fn set_icon(application: &NSApplication, image: &NSImage) {
    // SAFETY: The binding's only extra requirement is that the image must not
    // be nil. The caller supplies a successfully decoded, retained NSImage.
    unsafe { application.setApplicationIconImage(Some(image)) };
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_application_icon_decodes_without_external_resources() {
        let image = super::application_icon().expect("embedded PNG must decode in AppKit");
        assert!(image.isValid());
        assert_eq!(image.size().width, 1024.0);
        assert_eq!(image.size().height, 1024.0);
    }
}

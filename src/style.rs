//! the shared look for every ui w material you colors round shapes and google sans like caelestia

use gtk4::prelude::*;

/// css every window gets before its own and it uses the palette colors from theme.rs
pub const BASE: &str = "
.sevenshell, .sevenshell * {
    font-family: \"Google Sans Flex\", \"Symbols Nerd Font\", sans-serif;
    -gtk-icon-shadow: none;
    text-shadow: none;
}
.sevenshell .icon {
    font-family: \"Material Symbols Rounded\";
    font-weight: normal;
    font-feature-settings: \"liga\" 1;
    font-variation-settings: \"FILL\" 0, \"wght\" 400, \"opsz\" 24;
}
.sevenshell .icon.filled { font-variation-settings: \"FILL\" 1, \"wght\" 400, \"opsz\" 24; }
.sevenshell tooltip, tooltip.background {
    background: @m3inverseSurface;
    color: @m3inverseOnSurface;
    border-radius: 8px;
    border: none;
    box-shadow: none;
}
.sevenshell tooltip label, tooltip.background label { color: @m3inverseOnSurface; font-size: 12px; }
.sevenshell popover > contents, .sevenshell popover > arrow {
    background: @m3surfaceContainer;
    color: @m3onSurface;
}
.sevenshell popover scrolledwindow, .sevenshell popover viewport,
.sevenshell popover listview, .sevenshell popover list, .sevenshell popover row {
    background: transparent;
    color: @m3onSurface;
}
.sevenshell popover row:hover, .sevenshell popover row:selected { background: @m3secondaryContainer; }
.sevenshell popover label, .sevenshell popover image { color: @m3onSurface; }
.sevenshell popover row:selected label, .sevenshell popover row:selected image { color: @m3onSecondaryContainer; }
";

/// gtk draws its own bits light or dark to match the palette instead of the system theme
pub fn match_mode(dark: bool) {
    if let Some(settings) = gtk4::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

/// a material symbols icon by its name like wifi or volume_up
pub fn icon(name: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(name));
    label.add_css_class("icon");
    label
}

/// mark a window as one of ours so it gets the shared look
pub fn adopt(window: &impl IsA<gtk4::Widget>) {
    window.add_css_class("sevenshell");
}

use anyhow::{Result, bail};

use crate::value::{Kind, Value};

/// A setting macpack knows how to read and write via `defaults`.
#[derive(Debug)]
pub struct Setting {
    /// Stable identifier used in profiles, e.g. `dock.autohide`.
    pub id: &'static str,
    pub category: &'static str,
    pub label: &'static str,
    pub domain: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    /// Stored per machine (`defaults -currentHost`).
    pub host: bool,
    /// Process to `killall` so the change takes effect.
    pub restart: Option<&'static str>,
    pub description: &'static str,
    pub allowed: Allowed,
}

/// Values a setting accepts beyond matching its kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Allowed {
    Any,
    Strings(&'static [&'static str]),
    Ints(&'static [i64]),
    /// Inclusive range for Int and Float settings.
    Range(f64, f64),
}

impl Setting {
    const fn host(mut self) -> Self {
        self.host = true;
        self
    }

    const fn one_of(mut self, values: &'static [&'static str]) -> Self {
        self.allowed = Allowed::Strings(values);
        self
    }

    const fn ints(mut self, values: &'static [i64]) -> Self {
        self.allowed = Allowed::Ints(values);
        self
    }

    const fn range(mut self, min: f64, max: f64) -> Self {
        self.allowed = Allowed::Range(min, max);
        self
    }

    /// Checks `value` against the setting's kind and allowed values,
    /// widening integers to floats where needed.
    pub fn validate(&self, value: Value) -> Result<Value> {
        let value = value.conform(self.kind)?;
        let ok = match (self.allowed, &value) {
            (Allowed::Any, _) => true,
            (Allowed::Strings(list), Value::String(s)) => list.contains(&s.as_str()),
            (Allowed::Ints(list), Value::Int(n)) => list.contains(n),
            (Allowed::Range(min, max), Value::Int(n)) => (min..=max).contains(&(*n as f64)),
            (Allowed::Range(min, max), Value::Float(f)) => (min..=max).contains(f),
            _ => true,
        };
        if !ok {
            bail!("{value} is not allowed; expected {}", self.allowed_text().unwrap_or_default());
        }
        Ok(value)
    }

    /// Human-readable allowed values, if restricted.
    pub fn allowed_text(&self) -> Option<std::string::String> {
        let join = |items: Vec<std::string::String>| items.join(", ");
        match self.allowed {
            Allowed::Any => None,
            Allowed::Strings(list) => Some(format!("one of {}", join(list.iter().map(|s| s.to_string()).collect()))),
            Allowed::Ints(list) => Some(format!("one of {}", join(list.iter().map(|n| n.to_string()).collect()))),
            Allowed::Range(min, max) => Some(format!("{min} to {max}")),
        }
    }

    /// The values the interface can cycle through: booleans or a fixed
    /// list, followed by unset. Empty when the value is free-form.
    pub fn choices(&self) -> Vec<Option<Value>> {
        let mut out: Vec<Option<Value>> = match (self.kind, self.allowed) {
            (Kind::Bool, _) => vec![Some(Value::Bool(true)), Some(Value::Bool(false))],
            (_, Allowed::Strings(list)) => list.iter().map(|s| Some(Value::String(s.to_string()))).collect(),
            (_, Allowed::Ints(list)) => list.iter().map(|n| Some(Value::Int(*n))).collect(),
            _ => return Vec::new(),
        };
        out.push(None);
        out
    }
}

#[allow(clippy::too_many_arguments)]
const fn s(
    id: &'static str,
    category: &'static str,
    label: &'static str,
    domain: &'static str,
    key: &'static str,
    kind: Kind,
    restart: Option<&'static str>,
    description: &'static str,
) -> Setting {
    Setting { id, category, label, domain, key, kind, host: false, restart, description, allowed: Allowed::Any }
}

const DOCK: Option<&str> = Some("Dock");
const FINDER: Option<&str> = Some("Finder");
const UI_SERVER: Option<&str> = Some("SystemUIServer");
const CONTROL_CENTER: Option<&str> = Some("ControlCenter");
const CC: &str = "com.apple.controlcenter";
const MENU_ITEM: &str = "18 shows it in the menu bar, 8 or 24 hides it; unset uses the system default.";
const MENU_ITEM_VALUES: &[i64] = &[8, 18, 24];
const GLOBAL: &str = "NSGlobalDomain";

const HOT_CORNER: &str = "Action: 0 none, 2 Mission Control, 3 app windows, 4 Desktop, 5 start screen saver, \
     6 disable screen saver, 10 display sleep, 11 Launchpad, 12 Notification Center, 13 lock screen, 14 Quick Note.";
const HOT_CORNER_ACTIONS: &[i64] = &[0, 1, 2, 3, 4, 5, 6, 10, 11, 12, 13, 14];
const HOT_CORNER_MOD: &str =
    "Modifier required for the hot corner: 0 none, 131072 Shift, 262144 Control, 524288 Option, 1048576 Command.";

use Kind::{Bool, Float, Int, Layout, String};

pub static CATALOG: &[Setting] = &[
    // Dock
    s("dock.autohide", "Dock", "Automatically hide the Dock", "com.apple.dock", "autohide", Bool, DOCK,
      "Hide the Dock until the pointer reaches the screen edge."),
    s("dock.autohide-delay", "Dock", "Auto-hide delay (seconds)", "com.apple.dock", "autohide-delay", Float, DOCK,
      "Delay before the hidden Dock appears. 0 shows it instantly.").range(0.0, 10.0),
    s("dock.autohide-time-modifier", "Dock", "Auto-hide animation speed", "com.apple.dock", "autohide-time-modifier", Float, DOCK,
      "Duration multiplier of the show/hide animation. 0 disables the animation.").range(0.0, 10.0),
    s("dock.tilesize", "Dock", "Icon size", "com.apple.dock", "tilesize", Int, DOCK,
      "Dock icon size in points (16–128).").range(16.0, 128.0),
    s("dock.magnification", "Dock", "Magnification", "com.apple.dock", "magnification", Bool, DOCK,
      "Magnify icons under the pointer."),
    s("dock.largesize", "Dock", "Magnified icon size", "com.apple.dock", "largesize", Int, DOCK,
      "Icon size in points when magnified (16–128).").range(16.0, 128.0),
    s("dock.orientation", "Dock", "Position on screen", "com.apple.dock", "orientation", String, DOCK,
      "One of: left, bottom, right.").one_of(&["left", "bottom", "right"]),
    s("dock.mineffect", "Dock", "Minimize effect", "com.apple.dock", "mineffect", String, DOCK,
      "One of: genie, scale, suck.").one_of(&["genie", "scale", "suck"]),
    s("dock.minimize-to-application", "Dock", "Minimize windows into app icon", "com.apple.dock", "minimize-to-application", Bool, DOCK,
      "Minimize windows into their application's icon instead of the right side of the Dock."),
    s("dock.show-recents", "Dock", "Show suggested and recent apps", "com.apple.dock", "show-recents", Bool, DOCK,
      "Show recently used apps that are not kept in the Dock."),
    s("dock.show-process-indicators", "Dock", "Show indicators for open apps", "com.apple.dock", "show-process-indicators", Bool, DOCK,
      "Show a dot under running applications."),
    s("dock.static-only", "Dock", "Show only open apps", "com.apple.dock", "static-only", Bool, DOCK,
      "Only show running applications in the Dock."),
    // Mission Control
    s("mission.mru-spaces", "Mission Control", "Rearrange Spaces by recent use", "com.apple.dock", "mru-spaces", Bool, DOCK,
      "Automatically rearrange Spaces based on most recent use."),
    s("mission.expose-group-apps", "Mission Control", "Group windows by application", "com.apple.dock", "expose-group-apps", Bool, DOCK,
      "Group windows by application in Mission Control."),
    s("mission.spans-displays", "Mission Control", "Displays share one Space", "com.apple.spaces", "spans-displays", Bool, None,
      "true means displays do NOT have separate Spaces. Takes effect after logout."),
    // Hot corners
    s("hotcorner.top-left", "Hot Corners", "Top-left action", "com.apple.dock", "wvous-tl-corner", Int, DOCK, HOT_CORNER).ints(HOT_CORNER_ACTIONS),
    s("hotcorner.top-left-modifier", "Hot Corners", "Top-left modifier", "com.apple.dock", "wvous-tl-modifier", Int, DOCK, HOT_CORNER_MOD),
    s("hotcorner.top-right", "Hot Corners", "Top-right action", "com.apple.dock", "wvous-tr-corner", Int, DOCK, HOT_CORNER).ints(HOT_CORNER_ACTIONS),
    s("hotcorner.top-right-modifier", "Hot Corners", "Top-right modifier", "com.apple.dock", "wvous-tr-modifier", Int, DOCK, HOT_CORNER_MOD),
    s("hotcorner.bottom-left", "Hot Corners", "Bottom-left action", "com.apple.dock", "wvous-bl-corner", Int, DOCK, HOT_CORNER).ints(HOT_CORNER_ACTIONS),
    s("hotcorner.bottom-left-modifier", "Hot Corners", "Bottom-left modifier", "com.apple.dock", "wvous-bl-modifier", Int, DOCK, HOT_CORNER_MOD),
    s("hotcorner.bottom-right", "Hot Corners", "Bottom-right action", "com.apple.dock", "wvous-br-corner", Int, DOCK, HOT_CORNER).ints(HOT_CORNER_ACTIONS),
    s("hotcorner.bottom-right-modifier", "Hot Corners", "Bottom-right modifier", "com.apple.dock", "wvous-br-modifier", Int, DOCK, HOT_CORNER_MOD),
    // Finder
    s("finder.show-hidden-files", "Finder", "Show hidden files", "com.apple.finder", "AppleShowAllFiles", Bool, FINDER,
      "Show dotfiles and other hidden files in Finder."),
    s("finder.show-extensions", "Finder", "Show all filename extensions", GLOBAL, "AppleShowAllExtensions", Bool, FINDER,
      "Always show file extensions."),
    s("finder.extension-warning", "Finder", "Warn before changing an extension", "com.apple.finder", "FXEnableExtensionChangeWarning", Bool, FINDER,
      "Ask for confirmation when renaming a file's extension."),
    s("finder.pathbar", "Finder", "Show path bar", "com.apple.finder", "ShowPathbar", Bool, FINDER,
      "Show the path bar at the bottom of Finder windows."),
    s("finder.statusbar", "Finder", "Show status bar", "com.apple.finder", "ShowStatusBar", Bool, FINDER,
      "Show the status bar at the bottom of Finder windows."),
    s("finder.view-style", "Finder", "Default view style", "com.apple.finder", "FXPreferredViewStyle", String, FINDER,
      "Nlsv = list, icnv = icons, clmv = columns, glyv = gallery.").one_of(&["Nlsv", "icnv", "clmv", "glyv"]),
    s("finder.posix-title", "Finder", "Show full path in title", "com.apple.finder", "_FXShowPosixPathInTitle", Bool, FINDER,
      "Show the full POSIX path in Finder window titles."),
    s("finder.folders-first", "Finder", "Keep folders on top", "com.apple.finder", "_FXSortFoldersFirst", Bool, FINDER,
      "Sort folders before files in windows sorted by name."),
    s("finder.search-scope", "Finder", "Default search scope", "com.apple.finder", "FXDefaultSearchScope", String, FINDER,
      "SCev = this Mac, SCcf = current folder, SCsp = previous scope.").one_of(&["SCev", "SCcf", "SCsp"]),
    s("finder.new-window-target", "Finder", "New window location", "com.apple.finder", "NewWindowTarget", String, FINDER,
      "PfHm = home, PfDe = Desktop, PfDo = Documents, PfCm = computer, PfAF = recents.").one_of(&["PfHm", "PfDe", "PfDo", "PfCm", "PfAF", "PfLo"]),
    s("finder.desktop-icons", "Finder", "Show icons on Desktop", "com.apple.finder", "CreateDesktop", Bool, FINDER,
      "Show files and drives on the Desktop."),
    s("finder.no-ds-store-network", "Finder", "No .DS_Store on network volumes", "com.apple.desktopservices", "DSDontWriteNetworkStores", Bool, FINDER,
      "Avoid creating .DS_Store files on network shares."),
    // Keyboard
    s("keyboard.key-repeat", "Keyboard", "Key repeat rate", GLOBAL, "KeyRepeat", Int, None,
      "Lower is faster (System Settings minimum is 2). Takes effect after logout."),
    s("keyboard.initial-key-repeat", "Keyboard", "Delay until repeat", GLOBAL, "InitialKeyRepeat", Int, None,
      "Lower is shorter (System Settings minimum is 15). Takes effect after logout."),
    s("keyboard.press-and-hold", "Keyboard", "Press and hold for accents", GLOBAL, "ApplePressAndHoldEnabled", Bool, None,
      "false enables key repeat instead of the accent popup. Restart apps to apply."),
    s("keyboard.autocorrect", "Keyboard", "Correct spelling automatically", GLOBAL, "NSAutomaticSpellingCorrectionEnabled", Bool, None,
      "Automatic spelling correction in text fields."),
    s("keyboard.auto-capitalize", "Keyboard", "Capitalize words automatically", GLOBAL, "NSAutomaticCapitalizationEnabled", Bool, None,
      "Automatic capitalization in text fields."),
    s("keyboard.period-substitution", "Keyboard", "Double-space inserts period", GLOBAL, "NSAutomaticPeriodSubstitutionEnabled", Bool, None,
      "Add a period with a double-space."),
    s("keyboard.smart-quotes", "Keyboard", "Smart quotes", GLOBAL, "NSAutomaticQuoteSubstitutionEnabled", Bool, None,
      "Replace straight quotes with curly quotes."),
    s("keyboard.smart-dashes", "Keyboard", "Smart dashes", GLOBAL, "NSAutomaticDashSubstitutionEnabled", Bool, None,
      "Replace double hyphens with dashes."),
    s("keyboard.full-keyboard-access", "Keyboard", "Keyboard navigation", GLOBAL, "AppleKeyboardUIMode", Int, None,
      "2 or 3 lets Tab move focus between all controls; 0 only text fields and lists.").ints(&[0, 1, 2, 3]),
    s("keyboard.fn-keys", "Keyboard", "Use F1, F2… as standard keys", GLOBAL, "com.apple.keyboard.fnState", Bool, None,
      "Function keys act as F-keys without holding fn. Takes effect after logout."),
    // Trackpad
    s("trackpad.tap-to-click", "Trackpad", "Tap to click", "com.apple.AppleMultitouchTrackpad", "Clicking", Bool, None,
      "Tap with one finger to click. Takes effect after logout."),
    s("trackpad.three-finger-drag", "Trackpad", "Three-finger drag", "com.apple.AppleMultitouchTrackpad", "TrackpadThreeFingerDrag", Bool, None,
      "Drag windows and selections with three fingers. Takes effect after logout."),
    s("trackpad.natural-scrolling", "Trackpad", "Natural scrolling", GLOBAL, "com.apple.swipescrolldirection", Bool, None,
      "Content follows finger movement. Takes effect after logout."),
    s("trackpad.speed", "Trackpad", "Tracking speed", GLOBAL, "com.apple.trackpad.scaling", Float, None,
      "Pointer speed, 0 to 3. Takes effect after logout.").range(0.0, 3.0),
    // Screenshots
    s("screenshots.location", "Screenshots", "Save location", "com.apple.screencapture", "location", String, UI_SERVER,
      "Folder where screenshots are saved, e.g. ~/Desktop."),
    s("screenshots.type", "Screenshots", "File format", "com.apple.screencapture", "type", String, UI_SERVER,
      "One of: png, jpg, pdf, tiff, heic, gif, bmp.").one_of(&["png", "jpg", "pdf", "tiff", "heic", "gif", "bmp"]),
    s("screenshots.disable-shadow", "Screenshots", "No window shadow", "com.apple.screencapture", "disable-shadow", Bool, UI_SERVER,
      "Omit the drop shadow when capturing a window."),
    s("screenshots.show-thumbnail", "Screenshots", "Show floating thumbnail", "com.apple.screencapture", "show-thumbnail", Bool, UI_SERVER,
      "Show a thumbnail preview after taking a screenshot."),
    // Menu bar
    s("menubar.autohide", "Menu Bar", "Automatically hide the menu bar", GLOBAL, "_HIHideMenuBar", Bool, UI_SERVER,
      "Hide the menu bar until the pointer reaches the top of the screen."),
    s("menubar.clock-seconds", "Menu Bar", "Clock shows seconds", "com.apple.menuextra.clock", "ShowSeconds", Bool, UI_SERVER,
      "Display seconds in the menu bar clock."),
    s("menubar.clock-format", "Menu Bar", "Clock format", "com.apple.menuextra.clock", "DateFormat", String, UI_SERVER,
      "Date format string, e.g. \"EEE d MMM HH:mm\"."),
    s("menubar.show-wifi", "Menu Bar", "Show Wi-Fi", CC, "WiFi", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-bluetooth", "Menu Bar", "Show Bluetooth", CC, "Bluetooth", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-airdrop", "Menu Bar", "Show AirDrop", CC, "AirDrop", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-focus", "Menu Bar", "Show Focus", CC, "FocusModes", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-screen-mirroring", "Menu Bar", "Show Screen Mirroring", CC, "ScreenMirroring", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-display", "Menu Bar", "Show Display", CC, "Display", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-sound", "Menu Bar", "Show Sound", CC, "Sound", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-now-playing", "Menu Bar", "Show Now Playing", CC, "NowPlaying", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-keyboard-brightness", "Menu Bar", "Show Keyboard Brightness", CC, "KeyboardBrightness", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-battery", "Menu Bar", "Show Battery", CC, "Battery", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    s("menubar.show-spotlight", "Menu Bar", "Show Spotlight", CC, "Spotlight", Int, CONTROL_CENTER, MENU_ITEM)
      .ints(MENU_ITEM_VALUES).host(),
    // Control Center
    s("controlcenter.layout", "Control Center", "Control Center layout", "com.apple.controlcenter.bentoboxes", "boxes", Layout, CONTROL_CENTER,
      "Tiles as \"<Module> <W>x<H> at <X>,<Y>\", separated by \"; \" on a 4-column grid. macOS 26 or later.").host(),
    // Appearance
    s("appearance.style", "Appearance", "Dark mode", GLOBAL, "AppleInterfaceStyle", String, None,
      "\"Dark\" for dark mode; unset means light mode. Restart apps to apply.").one_of(&["Dark"]),
    s("appearance.accent-color", "Appearance", "Accent color", GLOBAL, "AppleAccentColor", Int, None,
      "-1 graphite, 0 red, 1 orange, 2 yellow, 3 green, 4 blue, 5 purple, 6 pink; unset means multicolor.").ints(&[-1, 0, 1, 2, 3, 4, 5, 6]),
    s("appearance.scrollbars", "Appearance", "Show scroll bars", GLOBAL, "AppleShowScrollBars", String, None,
      "One of: Automatic, WhenScrolling, Always.").one_of(&["Automatic", "WhenScrolling", "Always"]),
    // Windows & documents
    s("windows.save-to-cloud", "Windows & Documents", "Save new documents to iCloud", GLOBAL, "NSDocumentSaveNewDocumentsToCloud", Bool, None,
      "false makes save dialogs default to this Mac."),
    s("windows.expand-save-panel", "Windows & Documents", "Expanded save dialogs", GLOBAL, "NSNavPanelExpandedStateForSaveMode", Bool, None,
      "Open save dialogs in their expanded form."),
    s("windows.animations", "Windows & Documents", "Window open animations", GLOBAL, "NSAutomaticWindowAnimationsEnabled", Bool, None,
      "Animate opening windows and popovers."),
    s("windows.restore-on-reopen", "Windows & Documents", "Restore windows when reopening apps", GLOBAL, "NSQuitAlwaysKeepsWindows", Bool, None,
      "true restores windows when an app is reopened."),
    s("screensaver.idle-time", "Windows & Documents", "Screen saver delay (seconds)", "com.apple.screensaver", "idleTime", Int, None,
      "Seconds of inactivity before the screen saver starts; 0 disables it.").host(),
];

pub fn find(id: &str) -> Option<&'static Setting> {
    CATALOG.iter().find(|s| s.id == id)
}

/// Category names in catalog order.
pub fn categories() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for s in CATALOG {
        if !out.contains(&s.category) {
            out.push(s.category);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_prefixed() {
        let mut seen = HashSet::new();
        for s in CATALOG {
            assert!(seen.insert(s.id), "duplicate id {}", s.id);
            assert!(s.id.contains('.'), "id {} should be namespaced", s.id);
        }
        assert!(find("dock.autohide").is_some());
        assert!(find("nope").is_none());
    }

    #[test]
    fn validates_allowed_values() {
        let orientation = find("dock.orientation").unwrap();
        assert!(orientation.validate(Value::String("left".into())).is_ok());
        let err = orientation.validate(Value::String("top".into())).unwrap_err().to_string();
        assert!(err.contains("one of left, bottom, right"), "{err}");

        let size = find("dock.tilesize").unwrap();
        assert!(size.validate(Value::Int(64)).is_ok());
        assert!(size.validate(Value::Int(200)).is_err());
        let speed = find("trackpad.speed").unwrap();
        assert_eq!(speed.validate(Value::Int(2)).unwrap(), Value::Float(2.0));
        assert!(speed.validate(Value::Float(3.5)).is_err());
        assert!(find("hotcorner.top-left").unwrap().validate(Value::Int(7)).is_err());
        assert!(find("dock.autohide").unwrap().validate(Value::Int(1)).is_err());
    }

    #[test]
    fn allowed_values_match_kind() {
        for s in CATALOG {
            match s.allowed {
                Allowed::Strings(_) => assert_eq!(s.kind, Kind::String, "{}", s.id),
                Allowed::Ints(_) => assert_eq!(s.kind, Kind::Int, "{}", s.id),
                Allowed::Range(..) => assert!(matches!(s.kind, Kind::Int | Kind::Float), "{}", s.id),
                Allowed::Any => {}
            }
        }
        assert_eq!(find("dock.orientation").unwrap().choices().len(), 4);
        assert_eq!(find("dock.autohide").unwrap().choices().len(), 3);
        assert!(find("dock.tilesize").unwrap().choices().is_empty());
    }
}

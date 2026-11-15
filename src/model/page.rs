use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    DeviceSelect,
    Mixer,
    Settings,
    About,
    Manual,
}

impl fmt::Display for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match *self {
            Page::DeviceSelect => "Device",
            Page::Mixer => "Mixer",
            Page::Settings => "Settings",
            Page::About => "About",
            Page::Manual => "Manual",
        };
        write!(f, "{}", text)
    }
}

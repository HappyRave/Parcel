/// An app as Parcel lists it: one executable, however many processes run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// Full executable path; the app's identity.
    pub path: String,
    /// What the user sees, e.g. "Google Chrome".
    pub name: String,
}

impl App {
    /// Traffic whose owning process isn't known.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            path: String::new(),
            name: "Unknown app".to_owned(),
        }
    }
}

/// Looks up which app a process id belongs to (platform code).
pub trait Processes: Send {
    /// Always answers; a process that already exited gets a stand-in name.
    fn app(&mut self, pid: u32) -> App;
}

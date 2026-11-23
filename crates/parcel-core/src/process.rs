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
    /// Always answers. `image` is the executable path from the process's start event, when
    /// seen: it still names a process that has already exited. Without it, an exited process
    /// gets a stand-in name.
    fn app(&mut self, pid: u32, image: Option<&str>) -> App;
}

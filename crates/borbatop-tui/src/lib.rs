//! Terminal UI: rendering, keyboard handling and dialogs over immutable snapshots.
//!
//! The screen state ([`App`]) knows nothing about terminals: it turns [`Key`]s into
//! [`Command`]s and is drawn by [`draw`]. Only [`key_from_event`] touches terminal types.

mod alerts;
mod app;
mod dialog;
mod draw;
mod input;
mod model;
mod mouse;
mod terminal;
mod theme;
mod widgets;

pub use alerts::{Alert, AlertKind, AlertThresholds, evaluate};
pub use app::{App, MessageKind, UiConfig};
pub use draw::{WORDMARK, draw};
pub use input::{
    Command, ExportContent, ExportOrder, ExportRequest, Key, Seek, ServiceAction, SignalChoice,
    SortKey, Tab,
};
pub use model::{
    AurView, DiskRow, KernelView, Model, MountRow, NetRow, PacmanView, PressureView, ProcessRow,
    Report, ReportLine, ReportTone, StaleView,
};
pub use mouse::Mouse;
pub use terminal::{is_interrupt, key_from_event, mouse_from_event};
pub use theme::{Theme, ThemeKind};

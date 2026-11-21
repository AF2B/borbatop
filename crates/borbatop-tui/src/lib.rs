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
mod terminal;
mod theme;
mod widgets;

pub use alerts::{Alert, AlertKind, AlertThresholds, evaluate};
pub use app::{App, MessageKind, UiConfig};
pub use draw::{WORDMARK, draw};
pub use input::{Command, Key, ServiceAction, SignalChoice, SortKey, Tab};
pub use model::{
    DiskRow, KernelView, Model, MountRow, NetRow, PacmanView, PressureView, ProcessRow, StaleView,
};
pub use terminal::{is_interrupt, key_from_event};
pub use theme::{Theme, ThemeKind};

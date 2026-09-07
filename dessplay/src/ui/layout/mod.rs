//! Local, versioned terminal layout bundles. Controllers never live in templates.
mod compiler;
mod css;
mod reload;
mod renderer;
mod settings;
#[cfg(test)]
mod tests;
mod text;

pub use compiler::{Diagnostic, LayoutBundle, TemplateSchema};
pub(crate) use reload::Watcher;
pub(crate) use renderer::SplitRegion;
pub use renderer::{Presentation, PresentedRow, RenderedCollection, RenderedScene, Renderer};
pub use settings::LayoutSettings;
pub(crate) use text::wrap_body;
pub use text::{Fragment, measure_text};

/// Local startup options, never synced or written into user settings.
#[derive(Clone, Debug, Default)]
pub struct LayoutOptions {
    /// Explicit override directory, otherwise the platform config directory.
    pub directory: Option<std::path::PathBuf>,
    /// Ignore custom files at startup.
    pub builtin: bool,
}

/// Default local override directory; independent of replicated state.
pub fn default_directory() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| ".".into())
        .join("dessplay/ui")
}

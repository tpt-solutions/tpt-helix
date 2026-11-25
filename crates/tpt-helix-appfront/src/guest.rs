//! A guest component shown in AppFront. The guest's DOM is the page, and a click
//! dispatches the guest's handlers.
//!
//! The guest runs in the runtime's sandbox, with the grants it is given. Its page
//! is the HTML of its DOM (`RuntimeState::document_html`). A click on an element
//! runs that element's handlers, then its ancestors' handlers, as DOM events bubble.

use tpt_helix_runtime::capability::Capability;
use tpt_helix_runtime::wasm::{Host, Module, sandboxed_engine};

/// A running guest and the page it shows.
pub struct GuestSurface {
    instance: tpt_helix_runtime::wasm::Instance,
}

impl GuestSurface {
    /// Loads the component `bytes` as app `app`, grants it `caps`, and runs its
    /// `run` export.
    pub fn load(bytes: &[u8], app: &str, caps: Vec<Capability>) -> Result<Self, String> {
        let engine = sandboxed_engine().map_err(|e| e.to_string())?;
        let module = Module::load(&engine, bytes).map_err(|e| e.to_string())?;
        let mut instance = module
            .instantiate(&engine, Host::with_grants(app, caps))
            .map_err(|e| e.to_string())?;
        instance.run().map_err(|e| e.to_string())?;
        Ok(GuestSurface { instance })
    }

    /// The guest's current page, as HTML with `data-helix-id` on each element.
    pub fn html(&self) -> String {
        self.instance.host().state.document_html()
    }

    /// Dispatches a click on the guest element `element`. Its handlers run first,
    /// then its ancestors' handlers, nearest first. Returns the new page if any
    /// handler ran, since a handler may change the DOM.
    pub fn click(&mut self, element: u64) -> Result<Option<String>, String> {
        let state = &self.instance.host().state;
        let mut handlers = Vec::new();
        for id in std::iter::once(element).chain(state.ancestors(element)) {
            handlers.extend(state.click_handler_ids(id).unwrap_or_default());
        }
        if handlers.is_empty() {
            return Ok(None);
        }
        for handler in handlers {
            self.instance
                .dispatch_handler(handler)
                .map_err(|e| e.to_string())?;
        }
        Ok(Some(self.html()))
    }

    /// The host state, for inspecting what the guest stored or changed.
    pub fn host(&self) -> &Host {
        self.instance.host()
    }
}

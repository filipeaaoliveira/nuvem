//! The call a mutation will make, named once.
//!
//! # Why this exists
//!
//! The confirmation modal shows what is about to happen, and it is worth
//! reading only because what it shows is built by the same function that
//! builds what runs. While the transport was a command line that function
//! produced an argv and the modal rendered it, and three tests pinned the two
//! together -- one of them by running a real subprocess and comparing against
//! what the operating system received.
//!
//! A remote call has no argv, so the shared thing has to be the call itself.
//! This type is what both the request and the preview derive from, which is
//! the same property moved somewhere a transport without a command line can
//! still hold it.
//!
//! Pure data, and deliberately in the domain crate: it names what is being
//! asked for, not how to ask.

/// The field a patch sets, when the method is a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallField {
    pub name: &'static str,
    pub value: String,
}

/// A single mutating call: which service, which method, which resource, and
/// for a patch, what it sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCall {
    pub service: &'static str,
    pub method: &'static str,
    pub resource: String,
    pub field: Option<CallField>,
}

impl ResourceCall {
    pub fn new(service: &'static str, method: &'static str, resource: impl Into<String>) -> Self {
        Self {
            service,
            method,
            resource: resource.into(),
            field: None,
        }
    }

    /// The field this call sets.
    ///
    /// Only a patch has one, and Cloud SQL is why it exists: `start` and
    /// `stop` there are the same method on the same resource and differ in
    /// nothing else, so a preview that omitted the field would render both
    /// identically and tell an operator nothing about which they are
    /// confirming.
    pub fn with_field(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.field = Some(CallField {
            name,
            value: value.into(),
        });
        self
    }

    /// The method, qualified by its service: `compute.instances.stop`.
    pub fn summary(&self) -> String {
        format!("{}.{}", self.service, self.method)
    }

    /// The call as a confirmation renders it: one line, and not a long one.
    ///
    /// One line rather than several, and shorter than the command line it
    /// replaced, because both are constraints rather than preferences. The
    /// modal declines to draw at all when it cannot show everything it holds,
    /// so a preview that is taller or wider than what it replaced is a
    /// terminal size at which an operator can no longer confirm anything.
    /// Measured against the argv it replaces, this is shorter at every
    /// action.
    ///
    /// The project is dropped from the resource path because the modal states
    /// it on its own line directly above. What is left is the part that
    /// differs between one target and the next, which is the part worth
    /// reading twice.
    pub fn preview(&self) -> String {
        let resource = self.resource_within_project();
        let mut preview = format!("{} {resource}", self.summary());
        if let Some(field) = &self.field {
            let name = field.name.rsplit('.').next().unwrap_or(field.name);
            preview.push_str(&format!(" {name}={}", field.value));
        }
        preview
    }

    /// The resource path with its `projects/<id>/` prefix removed, or the
    /// whole path when it has none.
    fn resource_within_project(&self) -> &str {
        self.resource
            .split_once('/')
            .filter(|(head, _)| *head == "projects")
            .and_then(|(_, rest)| rest.split_once('/'))
            .map(|(_, rest)| rest)
            .unwrap_or(&self.resource)
    }
}

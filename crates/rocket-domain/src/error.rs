//! Domain errors (graph resolution failures).

/// Errors returned by pure domain logic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A target named neither a service nor a group.
    #[error("unknown service or group {target:?} in project {project}")]
    UnknownTarget { target: String, project: String },
    /// Services depend on each other in a loop; `services` lists every service
    /// that could not be ordered, sorted.
    #[error("dependency cycle between services: {}", .services.join(", "))]
    DependencyCycle { services: Vec<String> },
}

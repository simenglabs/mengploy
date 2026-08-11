pub mod model;
pub mod repo;

pub const EVENT_DEPLOYMENT_FAILED: &str = "deployment.failed";
pub const EVENT_DEPLOYMENT_RECOVERED: &str = "deployment.recovered";
pub const EVENT_DRIFT_DETECTED: &str = "reconciliation.drift_detected";
pub const EVENT_DRIFT_RESOLVED: &str = "reconciliation.drift_resolved";

pub const ALLOWED_EVENTS: [&str; 4] = [
    EVENT_DEPLOYMENT_FAILED,
    EVENT_DEPLOYMENT_RECOVERED,
    EVENT_DRIFT_DETECTED,
    EVENT_DRIFT_RESOLVED,
];

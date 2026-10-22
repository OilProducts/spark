#[path = "contracts/flow_install.rs"]
mod flow_install;
#[path = "process_contracts/workspace_live_sse_contracts.rs"]
mod workspace_live_sse_contracts;
#[allow(unused_imports)]
pub(crate) use flow_install::{approve_flow, install_flow};

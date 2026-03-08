//! Query nodes - access external geometry and data

use crate::ice::ops::{ExecutionContext, IceNode};

// ============================================================================
// GET TEMPLATE
// ============================================================================

/// Access template geometry from external context
/// 
/// This node provides access to a template geometry that was passed into
/// the ICE network from outside (via the subnet node's template input).
#[derive(Clone, Debug, Default)]
pub struct GetTemplate;

impl GetTemplate {
    pub fn new() -> Self {
        Self
    }
}

impl IceNode for GetTemplate {
    fn execute(&self, _ctx: &mut ExecutionContext) -> Result<(), String> {
        // GetTemplate doesn't need to do anything - the template is already
        // in ctx.external_geometry["template"] and CopyToPoints will access it
        // This node exists mainly for visual clarity in the graph
        Ok(())
    }

    fn name(&self) -> &str {
        "GetTemplate"
    }
}
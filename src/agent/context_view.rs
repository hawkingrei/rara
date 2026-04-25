use super::*;
use crate::context::SharedRuntimeContext;

impl Agent {
    pub fn shared_runtime_context(&self) -> SharedRuntimeContext {
        self.assemble_runtime_context()
    }
}

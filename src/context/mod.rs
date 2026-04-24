//! Staged context architecture module.
//!
//! Stage 1 introduces the shared assembly boundary while keeping the module
//! lightweight enough for branches that only need `mod context;` to compile.

mod assembler;
mod runtime;

pub use self::assembler::{AssembledContext, ContextAssembler, RuntimeContextInputs};
pub use self::runtime::{
    CompactionContextView, ContextBudgetView, PlanContextView, PromptContextView,
    SharedRuntimeContext,
};

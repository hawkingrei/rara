//! Staged context architecture module.
//!
//! Stage 1 introduces the shared assembly boundary while keeping the module
//! lightweight enough for branches that only need `mod context;` to compile.

mod assembler;
mod assembly_view;
mod compaction_view;
mod memory_selection;
mod retrieval_view;
mod runtime;

pub use self::assembler::{
    AssembledContext, AssembledTurnContext, ContextAssembler, RuntimeContextInputs,
    RuntimeInteractionInput,
};
pub use self::runtime::{
    CacheStatus, CompactionContextView, CompactionSourceContextEntry, ContextAssemblyEntry,
    ContextAssemblyView, ContextBudgetView, DropReason, MemorySelectionContextView,
    MemorySelectionItemContextEntry, PlanContextView, PromptContextView, PromptSourceContextEntry,
    RetrievalContextView, RetrievalSourceContextEntry, SharedRuntimeContext,
};

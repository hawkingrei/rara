#![allow(clippy::too_many_lines)]

use std::path::Path;

use crate::config::ConfigManager;
use crate::tui::state::{RuntimePhase, RuntimeSnapshot, TranscriptEntry, TranscriptTurn, TuiApp};
use insta::assert_snapshot;
use tempfile::tempdir;

use super::{ActiveCell, ActiveTurnCell, CommittedTurnCell, HistoryCell};

#[path = "cells_tests/active_general.rs"]
mod active_general;
#[path = "cells_tests/active_plan.rs"]
mod active_plan;
#[path = "cells_tests/committed.rs"]
mod committed;

// Semantic color tokens for the TUI.
//
// All render-layer code should use these constants instead of raw
// ratatui::style::Color values so that the color palette can be
// changed in one place and the visual hierarchy stays consistent.
use ratatui::style::Color;

// ── Text ────────────────────────────────────────────────────────
pub(crate) const TEXT_PRIMARY: Color = Color::White;
pub(crate) const TEXT_SECONDARY: Color = Color::DarkGray;
pub(crate) const TEXT_ACCENT: Color = Color::Cyan; // ✦ prefix, active labels
pub(crate) const TEXT_MUTED: Color = Color::Gray; // hint text

// ── User / Agent roles ──────────────────────────────────────────
pub(crate) const ROLE_PREFIX: Color = Color::Cyan; // both "› " and "✦ "

// ── Progress phases ─────────────────────────────────────────────
pub(crate) const PHASE_EXPLORING: Color = Color::Yellow;
pub(crate) const PHASE_EXPLORED: Color = Color::Rgb(231, 201, 92);
pub(crate) const PHASE_THINKING: Color = Color::LightBlue;
pub(crate) const PHASE_PLANNING: Color = Color::Cyan;
pub(crate) const PHASE_PLANNED: Color = Color::Cyan;
pub(crate) const PHASE_RUNNING: Color = Color::Yellow;
pub(crate) const PHASE_RAN: Color = Color::LightYellow;

// ── Status ──────────────────────────────────────────────────────
pub(crate) const STATUS_SUCCESS: Color = Color::LightGreen;
pub(crate) const STATUS_WARNING: Color = Color::Yellow;
pub(crate) const STATUS_ERROR: Color = Color::Red;
pub(crate) const STATUS_WORKING: Color = Color::Yellow;
pub(crate) const STATUS_READY: Color = Color::Green;
pub(crate) const STATUS_INFO: Color = Color::LightBlue;

// ── Interaction / approval ──────────────────────────────────────
pub(crate) const INTERACTION_APPROVAL: Color = Color::Cyan;
pub(crate) const INTERACTION_SHELL: Color = Color::Yellow;
pub(crate) const INTERACTION_INPUT: Color = Color::LightGreen;
pub(crate) const INTERACTION_QUEUED: Color = Color::DarkGray;

// ── UI surfaces ─────────────────────────────────────────────────
pub(crate) const SURFACE_BOTTOM_PANE_BG: Color = Color::Rgb(18, 20, 24);
pub(crate) const BORDER_DEFAULT: Color = Color::DarkGray;
pub(crate) const DIVIDER: Color = Color::DarkGray;

// ── Terminal output ─────────────────────────────────────────────
pub(crate) const TERMINAL_ACTIVE: Color = Color::Yellow;
pub(crate) const TERMINAL_STDOUT: Color = Color::DarkGray;
pub(crate) const TERMINAL_SUCCESS: Color = Color::LightGreen;
pub(crate) const TERMINAL_ERROR: Color = Color::Red;

// ── Badge / section label ───────────────────────────────────────
pub(crate) const BADGE_FG_DARK: Color = Color::White;
pub(crate) const BADGE_FG_LIGHT: Color = Color::Black;

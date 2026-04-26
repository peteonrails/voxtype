//! TUI application state.

use crate::setup::binary::{self, Acceleration, EngineFamily, Inventory, Variant};

/// What the event handler asks the run-loop to do next.
pub enum Action {
    None,
    Quit,
    SwitchVariant(Variant),
}

/// Result of the most recent variant-switch attempt, displayed as a banner.
pub struct SwitchOutcome {
    pub success: bool,
    pub message: String,
}

/// Layout of the variant matrix: rows = engine family, columns = acceleration.
pub const ROWS: &[EngineFamily] = &[EngineFamily::Whisper, EngineFamily::Onnx];
pub const COLS: &[Acceleration] = &[
    Acceleration::Avx2,
    Acceleration::Avx512,
    Acceleration::Vulkan,
    Acceleration::Cuda,
    Acceleration::Rocm,
];

pub struct App {
    pub inventory: Inventory,
    /// Cursor position in the variant matrix (row, col).
    pub cursor: (usize, usize),
    /// True after the active variant changes; daemon should be restarted.
    pub restart_needed: bool,
    /// Result of the last switch attempt, if any.
    pub last_switch: Option<SwitchOutcome>,
    pub daemon_running: bool,
}

impl App {
    pub fn new() -> Self {
        let inventory = binary::inventory();
        let cursor = initial_cursor(&inventory);
        Self {
            inventory,
            cursor,
            restart_needed: false,
            last_switch: None,
            daemon_running: is_daemon_running(),
        }
    }

    pub fn refresh_inventory(&mut self) {
        self.inventory = binary::inventory();
        self.daemon_running = is_daemon_running();
    }

    /// Map a (row, col) cell to a Variant if one exists for that combination.
    /// Returns None for invalid pairs (e.g. Whisper × CUDA).
    pub fn variant_at(&self, row: usize, col: usize) -> Option<Variant> {
        let family = *ROWS.get(row)?;
        let accel = *COLS.get(col)?;
        Variant::ALL
            .iter()
            .copied()
            .find(|v| v.family() == family && v.acceleration() == accel)
    }

    pub fn move_cursor(&mut self, drow: i32, dcol: i32) {
        let (r, c) = self.cursor;
        let new_r = clamp_signed(r as i32 + drow, ROWS.len());
        let new_c = clamp_signed(c as i32 + dcol, COLS.len());
        self.cursor = (new_r, new_c);
    }

    pub fn record_switch_attempt(&mut self, variant: Variant, result: Result<(), String>) {
        let (success, message) = match result {
            Ok(()) => (true, format!("Switched to {}.", variant.display())),
            Err(e) => (false, e),
        };
        if success {
            self.restart_needed = true;
        }
        self.last_switch = Some(SwitchOutcome { success, message });
        let _ = variant;
        self.refresh_inventory();
    }
}

fn clamp_signed(v: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    v.clamp(0, (len - 1) as i32) as usize
}

fn initial_cursor(inv: &Inventory) -> (usize, usize) {
    if let Some(active) = inv.active_variant {
        let row = ROWS.iter().position(|f| *f == active.family()).unwrap_or(0);
        let col = COLS
            .iter()
            .position(|a| *a == active.acceleration())
            .unwrap_or(0);
        (row, col)
    } else {
        (0, 0)
    }
}

/// Mirrors the check in main.rs; we duplicate it here to avoid a circular
/// dependency on a private helper.
fn is_daemon_running() -> bool {
    let pid_path = crate::config::Config::runtime_dir().join("pid");
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_at_finds_known_pairs() {
        let app = App::new();
        // Whisper × AVX2 = WhisperAvx2
        assert_eq!(app.variant_at(0, 0), Some(Variant::WhisperAvx2));
        // ONNX × CUDA = OnnxCuda
        assert_eq!(app.variant_at(1, 3), Some(Variant::OnnxCuda));
    }

    #[test]
    fn variant_at_returns_none_for_invalid_pairs() {
        let app = App::new();
        // Whisper × CUDA — no such variant
        assert_eq!(app.variant_at(0, 3), None);
        // ONNX × Vulkan — no such variant
        assert_eq!(app.variant_at(1, 2), None);
    }

    #[test]
    fn move_cursor_clamps_at_edges() {
        let mut app = App::new();
        app.cursor = (0, 0);
        app.move_cursor(-1, -1);
        assert_eq!(app.cursor, (0, 0));
        app.move_cursor(10, 10);
        assert_eq!(app.cursor, (ROWS.len() - 1, COLS.len() - 1));
    }
}

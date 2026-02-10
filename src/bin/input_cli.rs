//! A simple command-line tool to simulate mouse and keyboard input. This
//! binary is intended to be used by automated test scripts within the
//! self-programming agent. It wraps the [`enigo`](https://crates.io/crates/enigo)
//! crate to expose a few high-level subcommands for moving the cursor,
//! clicking, double-clicking, right-clicking, typing text, sending key
//! presses, and sleeping for a specified duration. The tool is kept
//! deliberately simple so that the language model can reliably invoke it
//! without needing to understand lower-level details of event dispatch.
//!
//! # Usage
//!
//! Compile the project with `cargo build --release`. This will produce
//! executables for the main application and this CLI. To run commands:
//!
//! ```sh
//! # Move the mouse to coordinates (100, 200)
//! ./target/release/input_cli move --x 100 --y 200
//! # Click the left mouse button at (300, 400)
//! ./target/release/input_cli click --x 300 --y 400
//! # Type the text "hello" at the current cursor location
//! ./target/release/input_cli type --text "hello"
//! # Press the Enter key
//! ./target/release/input_cli keypress --key enter
//! # Sleep for 2 seconds
//! ./target/release/input_cli sleep --ms 2000
//! ```

use clap::{Parser, Subcommand};
use enigo::{Enigo, KeyboardControllable, MouseButton, MouseControllable};
use enigo::Key;
use std::thread::sleep;
use std::time::Duration;

/// Top-level CLI definition. The user must specify exactly one subcommand.
#[derive(Parser)]
#[command(name = "input_cli", about = "Simulate mouse and keyboard actions")] 
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Supported subcommands for mouse and keyboard automation. Each variant
/// corresponds to a specific action. The variants and associated flag names
/// are chosen to be explicit and easy to use from shell scripts.
#[derive(Subcommand)]
enum Command {
    /// Move the mouse pointer to absolute screen coordinates (x, y).
    Move { x: i32, y: i32 },
    /// Click the left mouse button at the given coordinates. The cursor is
    /// moved first before clicking.
    Click { x: i32, y: i32 },
    /// Double-click the left mouse button at the given coordinates.
    DoubleClick { x: i32, y: i32 },
    /// Right-click at the given coordinates.
    RightClick { x: i32, y: i32 },
    /// Type a UTF-8 string at the current cursor location. Does not move
    /// the mouse.
    Type { text: String },
    /// Press a single key. Supported values include common key names such
    /// as "enter", "backspace", "tab", "esc", "space", and single
    /// printable characters (e.g. "a", "1").
    KeyPress { key: String },
    /// Sleep for a number of milliseconds. Useful for creating pauses
    /// between actions when scripting interactions.
    Sleep { ms: u64 },
    /// Drag the mouse from one coordinate to another while holding the
    /// left mouse button. Coordinates are absolute screen positions.
    Drag {
        #[arg(long = "from-x")]
        from_x: i32,
        #[arg(long = "from-y")]
        from_y: i32,
        #[arg(long = "to-x")]
        to_x: i32,
        #[arg(long = "to-y")]
        to_y: i32,
    },
    /// Hold down the left mouse button at a coordinate for a specified
    /// number of milliseconds. Useful for long press interactions.
    Hold {
        #[arg(long)]
        x: i32,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        ms: u64,
    },
    /// Press a combination of keys simultaneously (a keyboard shortcut).
    /// Provide a list of key names. Example: --keys ctrl shift s
    Shortcut {
        #[arg(required = true)]
        keys: Vec<String>,
    },
    /// Capture a screenshot of the primary display and write it to the
    /// specified output file path. The image is saved in PNG format. NOTE:
    /// Screenshot functionality has been disabled on this platform because the
    /// `screenshot` crate is not portable to all targets. Invoking this
    /// subcommand will produce an error.
    Screenshot { output: String },
}

fn main() {
    let cli = Cli::parse();
    let mut enigo = Enigo::new();
    match cli.command {
        Command::Move { x, y } => {
            enigo.mouse_move_to(x, y);
        }
        Command::Click { x, y } => {
            enigo.mouse_move_to(x, y);
            enigo.mouse_click(MouseButton::Left);
        }
        Command::DoubleClick { x, y } => {
            enigo.mouse_move_to(x, y);
            enigo.mouse_click(MouseButton::Left);
            enigo.mouse_click(MouseButton::Left);
        }
        Command::RightClick { x, y } => {
            enigo.mouse_move_to(x, y);
            enigo.mouse_click(MouseButton::Right);
        }
        Command::Type { text } => {
            // enigo handles unicode text sequences via key_sequence. It will
            // generate the appropriate key down/up events for each codepoint.
            enigo.key_sequence(&text);
        }
        Command::KeyPress { key } => {
            // Normalize the key string to lower case for matching. Then
            // translate to an Enigo Key. If an unknown key is provided,
            // print an error to stderr and exit silently.
            let lower = key.to_lowercase();
            let k: Option<Key> = match lower.as_str() {
                "enter" => Some(Key::Return),
                "backspace" => Some(Key::Backspace),
                "tab" => Some(Key::Tab),
                "esc" | "escape" => Some(Key::Escape),
                "space" => Some(Key::Space),
                "shift" => Some(Key::Shift),
                "ctrl" | "control" => Some(Key::Control),
                "alt" => Some(Key::Alt),
                // Single ASCII character: treat as a unicode layout key
                _ if lower.chars().count() == 1 => {
                    lower.chars().next().map(Key::Layout)
                }
                _ => None,
            };
            if let Some(key_code) = k {
                enigo.key_click(key_code);
            } else {
                eprintln!("Unsupported key: {}", key);
            }
        }
        Command::Sleep { ms } => {
            sleep(Duration::from_millis(ms));
        }
        Command::Drag { from_x, from_y, to_x, to_y } => {
            enigo.mouse_move_to(from_x, from_y);
            // Press and hold left button
            enigo.mouse_down(MouseButton::Left);
            enigo.mouse_move_to(to_x, to_y);
            enigo.mouse_up(MouseButton::Left);
        }
        Command::Hold { x, y, ms } => {
            enigo.mouse_move_to(x, y);
            enigo.mouse_down(MouseButton::Left);
            sleep(Duration::from_millis(ms));
            enigo.mouse_up(MouseButton::Left);
        }
        Command::Shortcut { keys } => {
            // Map each provided string to an Enigo Key. We press all down
            // first then release in reverse order to simulate common
            // keyboard shortcuts. Unsupported keys are skipped with a warning.
            let mut key_codes: Vec<Key> = Vec::new();
            for kstr in &keys {
                let lower = kstr.to_lowercase();
                let code: Option<Key> = match lower.as_str() {
                    "enter" => Some(Key::Return),
                    "backspace" => Some(Key::Backspace),
                    "tab" => Some(Key::Tab),
                    "esc" | "escape" => Some(Key::Escape),
                    "space" => Some(Key::Space),
                    "shift" => Some(Key::Shift),
                    "ctrl" | "control" => Some(Key::Control),
                    "alt" => Some(Key::Alt),
                    _ if lower.chars().count() == 1 => {
                        lower.chars().next().map(Key::Layout)
                    }
                    _ => None,
                };
                if let Some(c) = code {
                    key_codes.push(c);
                } else {
                    eprintln!("Unsupported key in shortcut: {}", kstr);
                }
            }
            for kc in &key_codes {
                enigo.key_down(kc.clone());
            }
            // release in reverse order
            for kc in key_codes.iter().rev() {
                enigo.key_up(kc.clone());
            }
        }
        Command::Screenshot { output } => {
            // Screenshot functionality is disabled on this platform. The original
            // implementation used the `screenshot` crate, which only works on
            // macOS and Windows and requires nightly Rust. On other platforms
            // (including Linux), this subcommand simply prints an error.
            let _ = output; // suppress unused variable warning
            eprintln!("Screenshot feature is not available on this platform");
        }
    }
}
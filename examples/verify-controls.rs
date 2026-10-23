//! Convert a physical-input test route with the same mapper used by Desktop.
//! Then replay the resulting GBA input and compare it to the original route.
#[allow(dead_code)]
#[path = "../src/control_context.rs"]
mod control_context;
#[allow(dead_code)]
#[path = "../src/controls.rs"]
mod controls;
use control_context::Context;
use controls::{Binding, Controls, InputLatch};
use std::{collections::BTreeMap, fs, io::Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "Usage: verify-controls CONTROLS.cfg CONTEXT.csv PHYSICAL.txt OUTPUT.gba-input".into(),
        );
    }
    let controls = Controls::load(std::path::Path::new(&args[0]))?;
    let mut changes = BTreeMap::new();
    for raw in fs::read_to_string(&args[2])?.lines() {
        let raw = raw.split('#').next().unwrap().trim();
        if raw.is_empty() {
            continue;
        }
        let (frame, names) = raw.split_once(' ').ok_or("Expected frame keys")?;
        let mut held = Vec::new();
        if names != "None" {
            for name in names.split(',') {
                held.push(match name.trim() {
                    "MouseLeft" => Binding::MouseLeft,
                    "MouseRight" => Binding::MouseRight,
                    "MouseMiddle" => Binding::MouseMiddle,
                    name => Binding::Key(
                        sdl2::keyboard::Scancode::from_name(name).ok_or("Unknown physical key")?,
                    ),
                });
            }
        }
        changes.insert(frame.parse::<u64>()?, held);
    }
    let mut output = fs::File::create_new(&args[3])?;
    writeln!(output, "gba-input v1")?;
    let mut latch = InputLatch::default();
    let mut previous = None;
    for row in fs::read_to_string(&args[1])?.lines().skip(1) {
        let fields: Vec<_> = row.split(',').collect();
        let frame = fields[0].parse::<u64>()?;
        let context = match fields[1] {
            "OnFoot" => Context::OnFoot,
            "Vehicle" => Context::Vehicle,
            "Menu" => Context::Menu,
            _ => return Err("Unknown context".into()),
        };
        let held = changes
            .range(..=frame)
            .next_back()
            .map(|(_, held)| held.as_slice())
            .unwrap_or(&[]);
        let buttons = latch.sample(&controls, context, |key| held.contains(&key));
        if previous != Some(buttons) {
            writeln!(output, "{frame} {buttons:03x}")?;
            previous = Some(buttons);
        }
    }
    Ok(())
}

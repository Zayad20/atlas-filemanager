//! Headless driver for rclone's interactive config state machine.
//!
//! OAuth backends (Google Drive, OneDrive, …) can't be configured with a single
//! `config/create` call — rclone walks a series of questions, and at the auth
//! step opens the browser + captures the token via its loopback server. We drive
//! that machine non-interactively, answering each prompt with its default, so
//! the only thing the user does is approve access in the browser.

use anyhow::{bail, Context};
use serde_json::Value;

use super::rc_api::RcClient;

/// Run the config machine to completion. Blocks (awaits) across the browser
/// OAuth step, which rclone performs itself.
pub async fn run_config_machine(
    rc: &RcClient,
    name: &str,
    backend_type: &str,
    parameters: Value,
) -> anyhow::Result<()> {
    let mut state = String::new();
    let mut result = String::new();
    // Bounded so a misbehaving/looping backend can't spin forever.
    for _ in 0..64 {
        let resp = rc
            .config_create_step(name, backend_type, parameters.clone(), &state, &result)
            .await
            .context("config/create step")?;

        let err = resp.get("Error").and_then(Value::as_str).unwrap_or("");
        if !err.is_empty() {
            bail!("{err}");
        }
        let next_state = resp.get("State").and_then(Value::as_str).unwrap_or("");
        if next_state.is_empty() {
            return Ok(()); // finished
        }
        result = default_answer(resp.get("Option"));
        state = next_state.to_string();
    }
    bail!("rclone config did not converge")
}

/// Choose the default answer for a config question: its `Default`, else the
/// first example value, else empty. This accepts every "press enter" prompt and
/// the "use auto config? (yes)" OAuth prompt without user input.
fn default_answer(option: Option<&Value>) -> String {
    let Some(opt) = option.filter(|v| !v.is_null()) else {
        return String::new();
    };
    match opt.get("Default") {
        Some(Value::String(s)) => return s.clone(),
        Some(Value::Bool(b)) => return b.to_string(),
        Some(Value::Number(n)) => return n.to_string(),
        _ => {}
    }
    if let Some(first) = opt
        .get("Examples")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        if let Some(v) = first.get("Value").and_then(Value::as_str) {
            return v.to_string();
        }
    }
    String::new()
}

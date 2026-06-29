use anyhow::{Context, Result};
use atlas_model::app_state::BgResult;
use extism::{Function, Manifest, Plugin, Val, ValType, Wasm};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize)]
struct RunRequest {
    cmd: String,
    args: Vec<String>,
}

#[derive(Serialize)]
struct RunResponse {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

pub struct WasmPluginManager {
    plugin_bytes: Vec<u8>,
    data_dir: PathBuf,
    bg_tx: flume::Sender<BgResult>,
    tools: HashMap<String, PathBuf>,
}

impl WasmPluginManager {
    pub fn new(
        wasm_bytes: Vec<u8>,
        data_dir: PathBuf,
        bg_tx: flume::Sender<BgResult>,
        tools: HashMap<String, PathBuf>,
    ) -> Self {
        #[allow(clippy::disallowed_methods)] // reason: callers run this inside spawn_blocking
        std::fs::create_dir_all(&data_dir).ok();
        Self {
            plugin_bytes: wasm_bytes,
            data_dir,
            bg_tx,
            tools,
        }
    }

    fn create_plugin(&self) -> Result<Plugin> {
        let wasm = Wasm::data(self.plugin_bytes.clone());
        let mut manifest = Manifest::new([wasm]);

        manifest.allowed_hosts = Some(vec![
            "open-vsx.org".to_string(),
            "*.open-vsx.org".to_string(),
        ]);

        manifest.allowed_paths = Some(std::collections::BTreeMap::from([(
            self.data_dir.to_string_lossy().to_string(),
            std::path::PathBuf::from("/ext"),
        )]));

        let bg_tx_clone = self.bg_tx.clone();
        let tools = self.tools.clone();

        let show_quick_pick = Function::new(
            "host_show_quick_pick",
            [ValType::I64],
            [ValType::I64],
            extism::UserData::new(()),
            move |plugin, inputs, outputs, _user_data| {
                // Parse the JSON list coming from WASM
                let offset = inputs[0].unwrap_i64() as u64;
                let handle = plugin
                    .memory_handle(offset)
                    .context("Invalid memory handle")?;
                let json_options = plugin.memory_str(handle)?;
                tracing::info!(
                    "Extensión pide mostrar lista en Command Bar: {}",
                    json_options
                );

                let items: Vec<String> = serde_json::from_str(json_options).unwrap_or_default();

                // Create a sync channel to wait for the UI
                let (reply_tx, reply_rx) = std::sync::mpsc::channel();

                // Send the request to GPUI's background worker listener
                let _ = bg_tx_clone.send(BgResult::ExtShowQuickPick { items, reply_tx });

                // BLOCK the WASM execution until the user clicks an option in GPUI
                let selection = reply_rx
                    .recv_timeout(std::time::Duration::from_secs(30))
                    .unwrap_or_default();

                let out_handle = plugin.memory_new(selection)?;
                outputs[0] = Val::I64(out_handle.offset() as i64);
                Ok(())
            },
        );

        let run_command = Function::new(
            "host_run_command",
            [ValType::I64],
            [ValType::I64],
            extism::UserData::new(()),
            move |plugin, inputs, outputs, _user_data| {
                let offset = inputs[0].unwrap_i64() as u64;
                let handle = plugin
                    .memory_handle(offset)
                    .context("host_run_command: invalid memory handle")?;
                let json_req = plugin.memory_str(handle)?;
                let req: RunRequest = serde_json::from_str(json_req)
                    .context("host_run_command: invalid request JSON")?;

                #[allow(clippy::disallowed_methods)] // reason: already inside spawn_blocking
                let program = tools
                    .get(&req.cmd)
                    .map_or_else(|| PathBuf::from(&req.cmd), PathBuf::clone);
                let output = std::process::Command::new(program).args(&req.args).output();

                let resp = match output {
                    Ok(out) => RunResponse {
                        exit_code: out.status.code().unwrap_or(-1),
                        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                    },
                    Err(e) => RunResponse {
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: e.to_string(),
                    },
                };

                let resp_json =
                    serde_json::to_string(&resp).context("host_run_command: serialize failed")?;
                let out_handle = plugin.memory_new(resp_json)?;
                outputs[0] = Val::I64(out_handle.offset() as i64);
                Ok(())
            },
        );

        let plugin = Plugin::new(&manifest, [show_quick_pick, run_command], true)
            .context("Error instanciando plugin WASM")?;

        Ok(plugin)
    }

    pub fn execute_command(&self, cmd_id: &str, payload_json: &str) -> Result<String> {
        #[derive(Serialize)]
        struct CommandExecute {
            command_id: String,
            payload: String,
            host_dir: String,
        }

        let mut plugin = self.create_plugin()?;
        let req = serde_json::to_string(&CommandExecute {
            command_id: cmd_id.to_string(),
            payload: payload_json.to_string(),
            host_dir: self.data_dir.to_string_lossy().to_string(),
        })?;

        let res_json = plugin
            .call::<&str, String>("execute_command", &req)
            .context(format!("Error ejecutando el comando WASM: {cmd_id}"))?;

        Ok(res_json)
    }

    pub fn execute_preview(&self, payload_json: &str) -> Result<String> {
        let mut plugin = self.create_plugin()?;
        plugin
            .call::<&str, String>("execute_preview", payload_json)
            .context("Error ejecutando preview WASM")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore = "requires icon-manager extension installed locally"]
    #[cfg(target_os = "windows")]
    fn test_execute_wasm() {
        let path = PathBuf::from(std::env::var("APPDATA").unwrap())
            .join("atlas\\extensions\\icon-manager\\plugin.wasm");
        let bytes = std::fs::read(&path).unwrap();
        let (tx, rx) = flume::unbounded();
        let manager = WasmPluginManager::new(
            bytes,
            path.parent().unwrap().to_path_buf(),
            tx,
            HashMap::new(),
        );

        std::thread::spawn(move || {
            if let Ok(BgResult::ExtShowQuickPick { items, reply_tx }) = rx.recv() {
                println!("WASM requested quick pick with items: {:?}", items);
                reply_tx.send(items[0].clone()).unwrap();
            }
        });

        match manager.execute_command("icon-manager.download-theme", "") {
            Ok(res) => println!("SUCCESS: {}", res),
            Err(e) => println!("ERROR: {:#}", e),
        }
    }
}

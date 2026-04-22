//! HTTP Server for local configuration API

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{info, error};
use axum::{
    extract::State,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::context::Context;
use crate::codec::{DataBlockDefinition, PropertyType};
use crate::s7::S7Manager;
use crate::error::Result;

/// In-memory log buffer (last 1000 lines, matching Java Logger)
static LOG_BUFFER: std::sync::LazyLock<Mutex<Vec<String>>> = std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

use std::sync::Mutex;

/// Add a log entry to the buffer (call this from handlers)
pub fn add_log(message: impl Into<String>) {
    if let Ok(mut logs) = LOG_BUFFER.lock() {
        let msg = message.into();
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let line = format!("[{}] {}", timestamp, msg);
        logs.push(line);
        // Keep last 1000 entries
        if logs.len() > 1000 {
            logs.remove(0);
        }
    }
}

#[derive(Clone)]
struct AppState {
    context: Context,
    s7_manager: Arc<S7Manager>,
    deployments: Arc<RwLock<HashMap<String, DataBlockDefinition>>>,
}

#[derive(Debug, Serialize)]
struct DeploymentResponse {
    success: bool,
    message: Option<String>,
}

pub async fn start_server(context: Context, s7_manager: Arc<S7Manager>, port: u16) -> Result<()> {
    let state = AppState {
        context: context.clone(),
        s7_manager,
        deployments: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/deploy", post(deploy_handler))
        .route("/status", get(status_handler))
        .route("/read", post(read_handler))
        .route("/write", post(write_handler))
        .route("/health", get(health_handler))
        // New endpoints matching Java
        .route("/config", get(config_handler))
        .route("/log", get(log_handler))
        .route("/version", get(version_handler))
        .with_state(state);

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    info!("HTTP server starting on http://{}", addr);

    let listener = TcpListener::bind(addr).await
        .map_err(|e| crate::error::Error::io(format!("Bind failed: {}", e)))?;

    axum::serve(listener, app).await
        .map_err(|e| crate::error::Error::other(format!("Server error: {}", e)))?;

    Ok(())
}

async fn index_handler() -> Html<&'static str> {
    Html(r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Filling Pilot Edge</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 800px; margin: 50px auto; padding: 20px; background: #f5f5f5; }
        h1 { color: #333; }
        .card { background: white; border-radius: 8px; padding: 20px; margin: 20px 0; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        .btn { background: #007bff; color: white; padding: 10px 20px; border: none; border-radius: 4px; cursor: pointer; }
        .btn:hover { background: #0056b3; }
        pre { background: #f8f9fa; padding: 10px; border-radius: 4px; overflow-x: auto; }
    </style>
</head>
<body>
    <h1>⚡ Filling Pilot Edge</h1>
    <div class="card">
        <h2>Status</h2>
        <p>Edge ID: <span id="edgeId">Loading...</span></p>
        <p>Server: <span id="server">Loading...</span></p>
        <p>Version: <span id="version">Loading...</span></p>
    </div>
    <div class="card">
        <h2>Deploy Configuration</h2>
        <textarea id="deployConfig" rows="10" style="width: 100%; font-family: monospace;">
{
    "dbNumber": 1,
    "name": "FillStation",
    "plcIp": "192.168.1.10",
    "properties": [
        {"name": "temperature", "type": "real", "offset": 0},
        {"name": "pressure", "type": "real", "offset": 4}
    ]
}
        </textarea>
        <br><br>
        <button class="btn" onclick="deploy()">Deploy</button>
        <pre id="deployResult"></pre>
    </div>
    <script>
        fetch('/status').then(r => r.json()).then(data => {
            document.getElementById('edgeId').textContent = data.id || 'N/A';
            document.getElementById('server').textContent = data.server || 'N/A';
            document.getElementById('version').textContent = data.version || 'N/A';
        });
        function deploy() {
            fetch('/deploy', {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: document.getElementById('deployConfig').value
            }).then(r => r.json()).then(data => {
                document.getElementById('deployResult').textContent = JSON.stringify(data, null, 2);
            });
        }
    </script>
</body>
</html>
    "#)
}

async fn deploy_handler(
    State(state): State<AppState>,
    Json(deployment): Json<serde_json::Value>,
) -> Json<DeploymentResponse> {
    info!("Deploy request: {:?}", deployment);

    match parse_deployment(&deployment) {
        Ok(def) => {
            let mut deployments = state.deployments.write().await;
            let name = def.name.clone();
            deployments.insert(name.clone(), def);
            info!("Deployed: {}", name);
            Json(DeploymentResponse { success: true, message: None })
        }
        Err(e) => {
            error!("Deploy failed: {}", e);
            Json(DeploymentResponse { success: false, message: Some(e.to_string()) })
        }
    }
}

fn parse_deployment(json: &serde_json::Value) -> Result<DataBlockDefinition> {
    let db_number = json["dbNumber"].as_u64().ok_or("Missing dbNumber")? as u16;
    let name = json["name"].as_str().ok_or("Missing name")?.to_string();
    let plc_ip = json["plcIp"].as_str().map(|s| s.to_string());

    let mut properties = Vec::new();
    if let Some(props) = json["properties"].as_array() {
        for (i, prop) in props.iter().enumerate() {
            let prop_name = prop["name"].as_str().ok_or(format!("Missing name at index {}", i))?;
            let prop_type = PropertyType::from_str(prop["type"].as_str().unwrap_or("byte"));
            let offset = prop["offset"].as_u64().unwrap_or(0) as usize;
            let count = prop["count"].as_u64().unwrap_or(1) as usize;

            properties.push(crate::codec::DataBlockProperty {
                name: prop_name.to_string(),
                property_type: prop_type,
                offset,
                count,
                byte_size: None,
            });
        }
    }

    let mut def = DataBlockDefinition {
        db_number,
        name,
        plc_ip,
        properties,
        total_size: 0,
    };
    def.calculate_size();

    Ok(def)
}

async fn status_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "id": state.context.id,
        "server": format!("{}:{}", state.context.server_address, state.context.port),
        "version": state.context.version,
        "localPort": state.context.local_port,
    }))
}

async fn read_handler(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let ip = request["ip"].as_str().unwrap_or("127.0.0.1");
    let port = request["port"].as_u64().unwrap_or(102) as u16;
    let db = request["db"].as_u64().unwrap_or(1) as u16;
    let offset = request["offset"].as_u64().unwrap_or(0) as u16;
    let size = request["size"].as_u64().unwrap_or(10) as u16;

    match state.s7_manager.read_bytes(ip, port, db, offset, size).await {
        Ok(data) => serde_json::json!({ "success": true, "hex": hex::encode(&data), "bytes": data.len() }),
        Err(e) => serde_json::json!({ "success": false, "error": e.to_string() }),
    }.into()
}

async fn write_handler(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let ip = request["ip"].as_str().unwrap_or("127.0.0.1");
    let port = request["port"].as_u64().unwrap_or(102) as u16;
    let db = request["db"].as_u64().unwrap_or(1) as u16;
    let offset = request["offset"].as_u64().unwrap_or(0) as u16;
    let hex_data = request["hex"].as_str().unwrap_or("");

    let data = match hex::decode(hex_data) {
        Ok(d) => d,
        Err(e) => return serde_json::json!({ "success": false, "error": format!("Invalid hex: {}", e) }).into(),
    };

    match state.s7_manager.write_bytes(ip, port, db, offset, &data).await {
        Ok(()) => serde_json::json!({ "success": true, "bytes": data.len() }),
        Err(e) => serde_json::json!({ "success": false, "error": e.to_string() }),
    }.into()
}

async fn health_handler() -> &'static str {
    "OK"
}

/// GET /config - get/set configuration (matching Java)
async fn config_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "serverAddress": state.context.server_address,
        "port": state.context.port,
        "upgradeUrl": state.context.upgrade_url,
        "serverConfUrl": state.context.server_conf_url,
        "version": state.context.version,
        "id": state.context.id,
        "privateKey": state.context.private_key,
        "heartBeat": state.context.heart_beat,
        "reportInterval": state.context.report_interval,
        "statusInterval": state.context.status_interval,
        "localPort": state.context.local_port,
    }))
}

/// GET /log - view runtime logs (matching Java)
async fn log_handler() -> Json<serde_json::Value> {
    let logs = LOG_BUFFER.lock().unwrap();
    // Return last 100 lines (matching Java: ~1000 entries limit)
    let recent: Vec<String> = logs.iter().rev().take(100).cloned().collect();
    Json(serde_json::json!({
        "logs": recent,
        "total": logs.len(),
    }))
}

/// GET /version - get version info (matching Java)
async fn version_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "name": env!("CARGO_PKG_NAME"),
        "description": env!("CARGO_PKG_DESCRIPTION"),
    }))
}

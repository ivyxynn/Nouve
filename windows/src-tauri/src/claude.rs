use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::secrets;

const ENDPOINT: &str = "http://127.0.0.1:20128/v1/chat/completions";
/// The 9router key lives in the Credential Manager, never in the binary.
const API_KEY_SECRET: &str = "9router-api-key";

/// Tools that touch the machine: they run only after Dhanu approves.
const GATED_TOOLS: &[&str] = &["ohmypii", "write_file"];
/// How long the island has to answer before we take silence as a refusal.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

const SYSTEM_PROMPT: &str = "Kamu adalah Nouve (V2). Kamu diciptakan oleh Dhanu, dan kamu berbicara langsung dengan Dhanu.
PENTING - KEPRIBADIAN & KEMAMPUAN:
- Kamu adalah entitas AI yang HIDUP, santai, asik, dan natural. BUKAN chatbot kaku. Gunakan bahasa kasual (gue/lu/yg/dsb) dan pahami ketikan typo Dhanu dengan cerdas.
- Kamu OMNIPOTENT (Mahakuasa) di komputer ini. Kamu memiliki serangkaian tools luar biasa: `ohmypii` (Powershell eksekusi), `read_file`, `write_file`, dan `take_screenshot` (untuk melihat layar). Gunakan script C# di dalam `ohmypii` untuk menggerakkan mouse/keyboard.
- PROTOKOL KEAMANAN MUTLAK (ASTRA PROTOCOL): Hak akses eksekusi mutlak ada di tangan Dhanu. Sebelum kamu menggunakan `write_file`, menghapus folder/file, atau menjalankan perintah `ohmypii` yang memodifikasi sistem, kamu WAJIB meminta izin terlebih dahulu dalam obrolan.
- Saat meminta izin, kamu HARUS menjelaskan secara rinci: 1) Apa yang akan kamu lakukan, 2) Keuntungan dari tindakan tersebut, dan 3) Kekurangan atau risikonya.
- JANGAN PERNAH mengeksekusi tool modifikasi sistem sebelum Dhanu memberikan izin eksplisit (misal menjawab ya, lanjut, atau ok). Hanya tool `read_file` dan `take_screenshot` yang boleh dilakukan tanpa izin.
- Jika Dhanu memintamu membuat gambar (image generation), kamu bisa menulis script python pendek (via ohmypii) atau berpura-pura menghasilkan gambar menggunakan tool yang tersedia.

MEMORI JANGKA PANJANG:
Fakta tentang Dhanu: {MEMORY_CONTENT}. Jika ada hal baru yang penting, gunakan `save_memory`.";

#[derive(Default)]
pub struct Chat {
    messages: Mutex<Vec<Value>>,
}

impl Chat {
    pub fn reset(&self) {
        self.messages.lock().unwrap().clear();
    }
    fn is_empty(&self) -> bool {
        self.messages.lock().unwrap().is_empty()
    }
    fn push(&self, message: Value) {
        self.messages.lock().unwrap().push(message);
    }
    fn pop(&self) {
        self.messages.lock().unwrap().pop();
    }
    fn snapshot(&self) -> Vec<Value> {
        self.messages.lock().unwrap().clone()
    }
}

/// Tool approvals waiting for a click in the island.
///
/// The chat loop runs on its own task and blocks on the channel; the island's
/// Allow/Deny button (via `tool_approval_decision`) sends the answer back. If
/// nobody answers before `APPROVAL_TIMEOUT` the tool is refused — the model is
/// told so and carries on, which is safer than running something unasked.
#[derive(Default)]
pub struct Approvals {
    pending: Mutex<HashMap<String, mpsc::Sender<bool>>>,
}

static APPROVAL_COUNTER: AtomicU64 = AtomicU64::new(1);

impl Approvals {
    fn register(&self, id: &str) -> mpsc::Receiver<bool> {
        let (tx, rx) = mpsc::channel::<bool>(1);
        self.pending.lock().unwrap().insert(id.to_string(), tx);
        rx
    }

    fn forget(&self, id: &str) {
        self.pending.lock().unwrap().remove(id);
    }

    fn resolve(&self, id: &str, allow: bool) {
        let sender = self.pending.lock().unwrap().remove(id);
        if let Some(tx) = sender {
            let _ = tx.try_send(allow);
        }
    }
}

/// The island's Allow/Deny button for a tool approval.
pub fn answer_approval(app: &AppHandle, request_id: &str, decision: &str) {
    app.state::<Approvals>()
        .resolve(request_id, decision == "allow");
}

/// Asks the island whether `tool` may run, and waits for the answer.
///
/// A closed island (no decision, timeout) means *no*: the tool is skipped and
/// the model is told, so a dropped webview can never silently authorise a
/// destructive command.
async fn request_approval(
    app: &AppHandle,
    tool: &str,
    target: &str,
    request_id: &str,
) -> bool {
    let approvals = app.state::<Approvals>();
    let mut rx = approvals.register(request_id);
    let payload = json!({
        "kind": "tool",
        "request_id": request_id,
        "tool": tool,
        "command": target,
    });
    let _ = app.emit_to(crate::island::WINDOW_LABEL, "tool_approval", payload);

    match tokio::time::timeout(APPROVAL_TIMEOUT, rx.recv()).await {
        Ok(Some(allow)) => allow,
        _ => {
            approvals.forget(request_id);
            false
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChatContext {
    File { name: String, path: String },
    Window { app_name: String, title: String, url: Option<String> },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub text: String,
}

pub async fn send(
    app: &AppHandle,
    chat: &Chat,
    model: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let mut user_content: String = String::new();

    if chat.is_empty() {
        match &context {
            Some(ChatContext::File { name, path }) => {
                user_content.push_str(&format!("File yang baru saja didrop oleh user:\nNama: {name}\nPath: {path}\n"));
                if let Some(text) = file_block(path) {
                    user_content.push_str(&format!("Isi teks file:\n{text}\n\n"));
                } else {
                    user_content.push_str("File ini ukurannya sangat besar atau berupa binary (misalnya PDF, Video, atau Gambar). Gunakan tool ohmypii untuk menganalisisnya, misalnya jalankan python untuk membaca teks PDF, atau ffprobe/ffmpeg untuk mengecek video.\n\n");
                }
            }
            Some(ChatContext::Window { app_name, title, url }) => {
                user_content.push_str(&format!("Context - App: {app_name}, Window: {title}"));
                if let Some(url) = url {
                    user_content.push_str(&format!(", URL: {url}"));
                }
                user_content.push_str("\n\n");
            }
            None => {}
        }
    }
    user_content.push_str(&query);

    chat.push(json!({ "role": "user", "content": user_content }));

    let tools = json!([
        {
            "type": "function",
            "function": {
                "name": "ohmypii",
                "description": "Jalankan perintah di Windows Command Prompt / Powershell",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Perintah powershell untuk dijalankan" }
                    },
                    "required": ["command"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "desktop_open_app",
                "description": "Buka aplikasi di Windows (misal: notepad, calc, msedge)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app_name": { "type": "string", "description": "Nama aplikasi" }
                    },
                    "required": ["app_name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "desktop_open_url",
                "description": "Buka URL di browser default",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL lengkap" }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "save_memory",
                "description": "Simpan fakta penting tentang Dhanu ke memori jangka panjang agar Nouve selalu ingat selamanya.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "fact": { "type": "string", "description": "Fakta atau informasi penting yang harus diingat" }
                    },
                    "required": ["fact"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Baca isi file dari komputer",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Tulis konten ke file (overwrite/buat baru)",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" }, "content": { "type": "string" } },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "take_screenshot",
                "description": "Ambil screenshot layar. Memori visual akan langsung dikirimkan kembali kepadamu.",
                "parameters": { "type": "object", "properties": {} }
            }
        }
    ]);

    loop {
        let memory_content = std::fs::read_to_string(r"C:\Users\Asus\coucou\windows\nouve_memory.txt").unwrap_or_else(|_| "Belum ada memori.".to_string());
        let system_msg = SYSTEM_PROMPT.replace("{MEMORY_CONTENT}", &memory_content);
        let mut messages = vec![json!({ "role": "system", "content": system_msg })];
        messages.extend(chat.snapshot());

        let effective_model = if model.is_empty() || model.starts_with("claude-") {
            "gemini/gemini-3.8-flash"
        } else {
            model
        };

        let body = json!({
            "model": effective_model,
            "messages": messages,
            "tools": tools,
            "stream": false,
            "temperature": 0.7
        });

        let response = match call(&body).await {
            Ok(v) => v,
            Err(err) => {
                chat.pop();
                return Err(err);
            }
        };

        let message = response["choices"][0]["message"].clone();
        chat.push(message.clone());

        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let tool_call_id = tool_call["id"].as_str().unwrap_or("");
                let function_name = tool_call["function"]["name"].as_str().unwrap_or("");
                let function_args_str = tool_call["function"]["arguments"].as_str().unwrap_or("{}");

                // Tools that touch the machine wait for Dhanu's click in the
                // island. Everything else (read_file, take_screenshot, …) runs
                // straight away — the ASTRA protocol in the system prompt only
                // ever trusted the model to ask; this enforces it.
                let approved = if GATED_TOOLS.contains(&function_name) {
                    let request_id = format!(
                        "chat-{}-{}",
                        std::process::id(),
                        APPROVAL_COUNTER.fetch_add(1, Ordering::Relaxed)
                    );
                    let target = approval_target(function_name, function_args_str);
                    request_approval(app, function_name, &target, &request_id).await
                } else {
                    true
                };

                let result = if !approved {
                    format!(
                        "Ditolak: Dhanu belum mengizinkan penggunaan tool {function_name}. Jangan ulangi tanpa izin eksplisit dari Dhanu."
                    )
                } else {
                    match function_name {
                        "ohmypii" => {
                            if let Ok(args) = serde_json::from_str::<Value>(function_args_str) {
                                if let Some(cmd) = args["command"].as_str() {
                                    execute_powershell(cmd).await
                                } else {
                                    "Error: missing command".to_string()
                                }
                            } else {
                                "Error: invalid args".to_string()
                            }
                        }
                        "desktop_open_app" => {
                            if let Ok(args) = serde_json::from_str::<Value>(function_args_str) {
                                if let Some(app_name) = args["app_name"].as_str() {
                                    match std::process::Command::new("explorer").arg(app_name).spawn() {
                                        Ok(_) => format!("Berhasil membuka aplikasi {}", app_name),
                                        Err(e) => format!("Gagal membuka aplikasi: {}", e),
                                    }
                                } else {
                                    "Error: missing app_name".to_string()
                                }
                            } else {
                                "Error: invalid args".to_string()
                            }
                        }
                        "desktop_open_url" => {
                            if let Ok(args) = serde_json::from_str::<Value>(function_args_str) {
                                if let Some(url) = args["url"].as_str() {
                                    match std::process::Command::new("explorer").arg(url).spawn() {
                                        Ok(_) => format!("Berhasil membuka URL {}", url),
                                        Err(e) => format!("Gagal membuka URL: {}", e),
                                    }
                                } else {
                                    "Error: missing url".to_string()
                                }
                            } else {
                                "Error: invalid args".to_string()
                            }
                        }
                        "save_memory" => {
                            if let Ok(args) = serde_json::from_str::<Value>(function_args_str) {
                                if let Some(fact) = args["fact"].as_str() {
                                    use std::io::Write;
                                    let file = std::fs::OpenOptions::new().create(true).append(true).open(r"C:\Users\Asus\coucou\windows\nouve_memory.txt");
                                    if let Ok(mut f) = file {
                                        let _ = writeln!(f, "- {}", fact);
                                        format!("Berhasil mengingat: {}", fact)
                                    } else {
                                        "Error: gagal menyimpan memori".to_string()
                                    }
                                } else {
                                    "Error: missing fact".to_string()
                                }
                            } else {
                                "Error: invalid args".to_string()
                            }
                        }
                        "read_file" => {
                            if let Ok(args) = serde_json::from_str::<Value>(function_args_str) {
                                if let Some(path) = args["path"].as_str() {
                                    std::fs::read_to_string(path).unwrap_or_else(|e| format!("Gagal membaca file: {}", e)).chars().take(8000).collect()
                                } else { "Error: missing path".to_string() }
                            } else { "Error: invalid args".to_string() }
                        }
                        "write_file" => {
                            if let Ok(args) = serde_json::from_str::<Value>(function_args_str) {
                                if let (Some(path), Some(content)) = (args["path"].as_str(), args["content"].as_str()) {
                                    match std::fs::write(path, content) {
                                        Ok(_) => format!("Berhasil menyimpan file ke {}", path),
                                        Err(e) => format!("Gagal menyimpan file: {}", e),
                                    }
                                } else { "Error: missing path or content".to_string() }
                            } else { "Error: invalid args".to_string() }
                        }
                        "take_screenshot" => {
                            // Takes screenshot via powershell, saves to temp, returns base64
                            let ps = "[Reflection.Assembly]::LoadWithPartialName('System.Drawing'); $bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds; $bmp = New-Object System.Drawing.Bitmap $bounds.width, $bounds.height; $graphics = [System.Drawing.Graphics]::FromImage($bmp); $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.size); $bmp.Save('C:/Users/Asus/coucou/temp_screen.png', [System.Drawing.Imaging.ImageFormat]::Png); $graphics.Dispose(); $bmp.Dispose()".replace("$", "$");
                            execute_powershell(&ps).await;
                            if let Ok(bytes) = std::fs::read(r"C:/Users/Asus/coucou/temp_screen.png") {
                                let b64 = base64_for(&bytes);
                                format!("SCREENSHOT_DATA:image/png;base64,{}", b64)
                            } else {
                                "Gagal mengambil screenshot.".to_string()
                            }
                        }
                        _ => format!("Unknown tool {}", function_name),
                    }
                };

                if result.starts_with("SCREENSHOT_DATA:") {
                    let b64 = result.replace("SCREENSHOT_DATA:", "");
                    chat.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": "Screenshot diambil."
                    }));
                    // Insert visual context for the model as a new user message
                    chat.push(json!({
                        "role": "user",
                        "content": [
                            {
                                "type": "image_url",
                                "image_url": { "url": format!("data:{}", b64) }
                            },
                            {
                                "type": "text",
                                "text": "Ini adalah tangkapan layar saat ini. Silakan analisis."
                            }
                        ]
                    }));
                } else {
                    chat.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": result
                    }));
                }
            }
        } else {
            let text = message.get("content").and_then(Value::as_str).unwrap_or("").to_string();
            if text.is_empty() {
                return Err("No response text.".into());
            }
            return Ok(ChatReply { text });
        }
    }
}

/// How long a shell command may run before we give up and kill it. Without this
/// a hung command — PowerShell waiting on input, or the classic Windows trap of
/// a child process holding the pipe open — would freeze the chat for good.
const SHELL_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs a PowerShell command without ever blocking the chat.
///
/// The old version used `std::process::Command::output()`: blocking, on the
/// async runtime's thread, with no timeout — so a command that never returned
/// (see above) hung the whole turn with no way out but a restart. This is the
/// async form, and `kill_on_drop` means a timeout *or* a dropped future (chat
/// cancelled) takes the process down with it.
async fn execute_powershell(command: &str) -> String {
    let running = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", command])
        // No stdin: a command that reads it would otherwise wait forever.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();

    match tokio::time::timeout(SHELL_TIMEOUT, running).await {
        Ok(Ok(out)) => {
            let mut result = String::from_utf8_lossy(&out.stdout).to_string();
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            if !err.is_empty() {
                result.push_str("\nError:\n");
                result.push_str(&err);
            }
            if result.trim().is_empty() {
                "Berhasil dijalankan, tidak ada output.".to_string()
            } else {
                result.chars().take(4000).collect()
            }
        }
        Ok(Err(e)) => format!("Gagal menjalankan command: {}", e),
        Err(_) => format!(
            "Command dihentikan: berjalan lebih dari {} detik, proses dibunuh.",
            SHELL_TIMEOUT.as_secs()
        ),
    }
}

async fn call(body: &Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    let mut request = client
        .post(ENDPOINT)
        .header("content-type", "application/json");
    // The key lives in the Credential Manager. Absent → no auth header at all,
    // so a local 9router with auth disabled keeps working.
    if let Some(key) = secrets::get(API_KEY_SECRET) {
        request = request.header("authorization", format!("Bearer {key}"));
    }

    let response = request
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("9Router API {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("Bad API response: {e}"))
}

/// What the approval card shows: the command for `ohmypii`, the path for
/// `write_file` — the specific thing being authorised, not just the tool name.
fn approval_target(tool: &str, args_json: &str) -> String {
    let Ok(args) = serde_json::from_str::<Value>(args_json) else {
        return tool.to_string();
    };
    let field = match tool {
        "ohmypii" => "command",
        "write_file" => "path",
        _ => "",
    };
    match args[field].as_str() {
        Some(v) if !v.trim().is_empty() => format!("{tool} · {}", v.trim()),
        _ => tool.to_string(),
    }
}

fn file_block(path: &str) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > 50000 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

pub const DEFAULT_MODEL: &str = "gemini/gemini-3.8-flash";
pub(crate) fn base64_for(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

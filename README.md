# zed-antigravity-bridge

A high-performance, ultra-lightweight Rust daemon that bridges Zed IDE's inline edit predictions (`open_ai_compatible_api`) with Google Cloud Code Assist / Google Antigravity (`daily-cloudcode-pa.googleapis.com`).

---

## Why This Exists

Zed IDE supports AI inline edit predictions (ghost text completions) via an OpenAI-compatible `/v1/completions` API. However, Google Cloud Code Assist / Antigravity utilizes:
- A proprietary Google Cloud envelope format (`v1internal:streamGenerateContent?alt=sse`).
- Dynamic OAuth 2.0 Bearer tokens with expiration.
- Server-Sent Events (SSE) streaming for generation chunks.

`zed-antigravity-bridge` acts as a local proxy daemon between Zed and Google Cloud Code:
1. Receives standard OpenAI-compatible FIM (Fill-In-the-Middle) requests from Zed on `127.0.0.1:8080/v1/completions`.
2. Automatically manages and caches Google OAuth tokens via the `omp` CLI (with automatic 401 token invalidation and refresh).
3. Translates FIM prompts (supporting StarCoder, CodeLlama, and DeepSeek delimiters) into Google Gemini Cloud Code payloads.
4. Streams and aggregates SSE chunks, sanitizes completions (stripping markdown fences and prefix echoes), and returns clean OpenAI JSON to Zed in ~800ms.
5. Consumes only **~5 MB of RAM** and negligible CPU when idle, compared to 200–500 MB typical of Node.js-based language server bridges.

---

## Prerequisites

- **Rust toolchain** (Rust 2021 edition, `cargo`).
- **`omp` CLI** available in `PATH` (or configured via `OMP_BIN`), authenticated with Google Antigravity.
- **Zed IDE** (v0.170+ or recent version with `edit_predictions` support).

---

## Building & Installation

Clone the repository and build the optimized release binary:

```bash
git clone https://github.com/your-username/zed-antigravity-bridge.git
cd zed-antigravity-bridge
cargo build --release
```

The compiled binary will be placed at `target/release/zed-antigravity-bridge`.

To install it directly into `~/.cargo/bin`:

```bash
cargo install --path .
```

---

## Running

### 1. Manual Execution

Run the binary directly:

```bash
zed-antigravity-bridge --port 8080 --model tab_flash_lite_preview
```

Available flags and environment variables:

| Flag | Environment Variable | Default | Description |
|---|---|---|---|
| `-p, --port` | `BRIDGE_PORT` | `8080` | Local TCP listening port |
| `--host` | `BRIDGE_HOST` | `127.0.0.1` | Local bind address |
| `-m, --model` | `DEFAULT_MODEL` | `tab_flash_lite_preview` | Default Google Antigravity completion model |
| `-u, --upstream` | `UPSTREAM_URL` | `https://daily-cloudcode-pa.googleapis.com` | Upstream Cloud Code endpoint |
| — | `OMP_BIN` | `omp` | Custom path to the `omp` binary |

### 2. Automatic Background Service (systemd user daemon)

On Linux systems running systemd, you can configure the bridge as a background user daemon:

1. Create `~/.config/systemd/user/zed-antigravity-bridge.service`:

```ini
[Unit]
Description=Zed Antigravity Autocompletion Bridge
After=network.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/zed-antigravity-bridge --port 8080 --model tab_flash_lite_preview
Restart=always
RestartSec=3
Environment=RUST_LOG=zed_antigravity_bridge=info

[Install]
WantedBy=default.target
```

*(Note: Replace `%h/.cargo/bin/zed-antigravity-bridge` with the absolute path to the binary if installed elsewhere).*

2. Enable and start the service:

```bash
systemctl --user daemon-reload
systemctl --user enable --now zed-antigravity-bridge.service
```

3. Manage the service:

```bash
# Check status
systemctl --user status zed-antigravity-bridge.service

# View live logs
journalctl --user -u zed-antigravity-bridge.service -f

# Restart daemon
systemctl --user restart zed-antigravity-bridge.service
```

---

## Zed IDE Configuration

Add the following to your Zed settings file (`settings.json`):

```json
{
  "show_edit_predictions": true,
  "edit_predictions": {
    "provider": "open_ai_compatible_api",
    "mode": "eager",
    "open_ai_compatible_api": {
      "api_url": "http://127.0.0.1:8080/v1/completions",
      "model": "tab_flash_lite_preview",
      "prompt_format": "infer",
      "max_output_tokens": 64,
      "prediction_debounce": 0
    }
  }
}
```

---

## Verification & Testing

### Health Check

Verify that the local bridge is responding:

```bash
curl -s http://127.0.0.1:8080/health
# Expected output: OK
```

### Mock FIM Completion Request

Send a sample Fill-in-the-Middle request to test end-to-end token generation:

```bash
curl -s -X POST http://127.0.0.1:8080/v1/completions \
  -H "Content-Type: application/json" \
  -d '{"prompt": "<fim_prefix>fn add(a: i32, b: i32) -> i32 {\n    <fim_suffix>\n}\n<fim_middle>", "max_tokens": 16}'
```

Expected response format:

```json
{
  "id": "cmpl-...",
  "object": "text_completion",
  "created": 1790184526,
  "model": "tab_flash_lite_preview",
  "choices": [
    {
      "text": "a + b\n}",
      "index": 0,
      "logprobs": null,
      "finish_reason": "STOP"
    }
  ]
}
```

---

## License

This project is licensed under the GNU Affero General Public License v3.0 (AGPLv3) — see the [LICENSE](LICENSE) file for details.

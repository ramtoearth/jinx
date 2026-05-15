# jinx

Jinx is a terminal-based day organizer. It manages tasks and events in your daily life using natural language.

![Demo](assets/demoCLItasks.gif)

## Quick install

### macOS and Linux

```bash
curl -fsSL https://raw.githubusercontent.com/ramtoearth/jinx/main/scripts/install.sh | bash
```

The script downloads the precompiled binary for your platform and installs `uv` if you don't have it.

### Windows

```powershell
irm https://raw.githubusercontent.com/ramtoearth/jinx/main/scripts/install.ps1 | iex
```

### Manual installation

Download the binary for your system from the [releases page](https://github.com/ramtoearth/jinx/releases/latest):

| System | File |
|--------|------|
| macOS Apple Silicon | `jinx-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `jinx-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| Linux x86-64 | `jinx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `jinx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x64 | `jinx-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

Each file includes a `.sha256` to verify download integrity.

## Getting started

Once installed, you need Ollama with a model that supports tool calling:

```bash
# Install Ollama (https://ollama.com)
ollama pull llama3.2:3b   # recommended model (~2 GB)

# Start jinx
jinx
```

The first launch takes ~30 seconds while `uv` installs the agent dependencies in an isolated environment. Subsequent launches are instant.

## Model configuration

Change the model directly from the app with **Ctrl+P**:

![Change model](assets/demoConfigModelo.gif)

Select the provider (Local/Remote), type the model name, and press Enter. The agent restarts automatically.

### Ollama (local)

Models with tool calling support: `llama3.1`, `llama3.2`, `qwen3`.

### Amazon Bedrock (remote)

1. Authenticate with AWS CLI v2 (version 2.32.0+):

```bash
aws login
```

This opens the browser to sign in. Credentials are renewed for up to 12 hours. See the [official documentation](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-sign-in.html).

2. Enable the model in Bedrock (Amazon Bedrock → Model access) in your region.

3. Open **Ctrl+P** in jinx, select Remote, and type the model ID (e.g., `anthropic.claude-3-5-sonnet-20241022-v2:0`).

## Language

Jinx defaults to English. To use it in Spanish, set `language = "es"` in your config file (`~/.config/jinx/config.toml`).

## Building from source

If you prefer to compile it yourself:

```bash
# Requirements: Rust (https://rustup.rs), uv (https://astral.sh/uv), Ollama

git clone https://github.com/ramtoearth/jinx
cd jinx
cargo install --path tui
```

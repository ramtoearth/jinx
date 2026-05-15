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

Select the provider (**Local** / **Remote**), choose a backend, type the model name, and press Enter. The agent restarts automatically.

### Local

#### Ollama

Runs entirely on your machine — no data leaves the device. Requires [Ollama](https://ollama.com) with a model that supports tool calling.

Models with tool calling support: `llama3.1`, `llama3.2`, `qwen3`.

### Remote

Remote providers send chat messages to an external service. Select **Remote** in Ctrl+P, then pick a backend with ←/→.

API keys are read from environment variables — add them to your shell profile (e.g., `~/.zshrc`):

```bash
export OPENAI_API_KEY="..."
export ANTHROPIC_API_KEY="..."
export GOOGLE_API_KEY="..."
export LLAMA_API_KEY="-..."
```

| Backend | Default model | Env variable | Setup |
|---------|--------------|--------------|-------|
| **Bedrock** | SDK default (region-aware) | AWS credentials (`aws login`) | Enable model in Amazon Bedrock → Model access |
| **OpenAI** | `gpt-4o` | `OPENAI_API_KEY` | [platform.openai.com](https://platform.openai.com) |
| **Anthropic** | `claude-sonnet-4-6` | `ANTHROPIC_API_KEY` | [console.anthropic.com](https://console.anthropic.com) |
| **Gemini** | `gemini-2.5-flash-lite` | `GOOGLE_API_KEY` | [aistudio.google.dev](https://aistudio.google.dev) |
| **LlamaAPI** | `Llama-4-Maverick-17B-128E-Instruct-FP8` | `LLAMA_API_KEY` | [llamaapi.com](https://www.llamaapi.com) |

You can type any model ID supported by each provider. For the full list of supported providers and model options, see the [Strands Agents model providers documentation](https://strandsagents.com/docs/user-guide/concepts/model-providers/).

## Language

Jinx defaults to English. Switch to Spanish from **Ctrl+P** (Language field). Currently supported languages are `English` and `Spanish`.

## Building from source

If you prefer to compile it yourself:

```bash
# Requirements: Rust (https://rustup.rs), uv (https://astral.sh/uv), Ollama

git clone https://github.com/ramtoearth/jinx
cd jinx
cargo install --path tui
```

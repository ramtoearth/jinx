# jinx

Aplicación de terminal para gestionar tareas, eventos y grupos usando lenguaje natural. El agente de IA corre localmente con [Ollama](https://ollama.com), tus datos no salen de tu máquina.

## Requisitos

| Herramienta | Instalación |
|-------------|-------------|
| [Rust](https://rustup.rs) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| [uv](https://astral.sh/uv) | `brew install uv` |
| [Ollama](https://ollama.com) | `brew install ollama` |


## Instalación

```bash
# 1. Clonar el repositorio
git clone https://github.com/tu-usuario/jinx
cd jinx

# 2. Descargar el modelo de IA
ollama pull llama3.1:latest

# 3. Instalar el binario
cargo install --path tui
```

Ejecuta la app desde cualquier directorio con:

```bash
jinx
```

> El primer arranque tarda ~30 segundos mientras `uv` instala las dependencias del agente en un entorno aislado. Los arranques posteriores son instantáneos.

## Configuración del modelo

Al ejecutar la app por primera vez se crea el archivo de configuración:

- **macOS:** `~/Library/Application Support/jinx/config.toml`
- **Linux:** `~/.config/jinx/config.toml`

```toml
# Proveedor: "local" (Ollama) o "remote" (Amazon Bedrock)
provider = "local"

[local]
# Modelos compatibles: llama3.1, llama3.2, qwen3
model = "llama3.1:latest"
host  = "http://localhost:11434"

[remote]
# ID del modelo en Amazon Bedrock
model_id = "anthropic.claude-3-5-sonnet-20241022-v2:0"
```

Edita el archivo y reinicia la app para aplicar los cambios.

## Dependencias principales

- [Strands Agents](https://strandsagents.com) — framework de agentes de IA en Python
- [strands-agents-tools](https://github.com/strands-agents/tools) — herramientas built-in (hora, archivos)
- [Ratatui](https://ratatui.rs) — interfaz de terminal en Rust
- [Ollama](https://ollama.com) — servidor local de modelos de lenguaje

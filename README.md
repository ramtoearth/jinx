# jinx

Organizador de día en terminal. Gestiona tareas, eventos y grupos usando lenguaje natural. El agente de IA corre localmente con [Ollama](https://ollama.com) — tus datos no salen de tu máquina.

![Demo](assets/demoCLItasks.gif)

## Instalación rápida

### macOS y Linux

```bash
curl -fsSL https://raw.githubusercontent.com/ramtoearth/jinx/main/install.sh | bash
```

El script descarga el binario precompilado para tu plataforma e instala `uv` si no lo tienes.

### Windows

```powershell
irm https://raw.githubusercontent.com/ramtoearth/jinx/main/install.ps1 | iex
```

### Instalación manual

Descarga el binario para tu sistema desde la [página de releases](https://github.com/ramtoearth/jinx/releases/latest):

| Sistema | Archivo |
|---------|---------|
| macOS Apple Silicon | `jinx-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `jinx-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| Linux x86-64 | `jinx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `jinx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x64 | `jinx-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

Cada archivo incluye un `.sha256` para verificar la integridad de la descarga.

## Primeros pasos

Una vez instalado, necesitas Ollama con un modelo que soporte llamadas a herramientas:

```bash
# Instala Ollama (https://ollama.com)
ollama pull llama3.2:3b   # modelo recomendado (~2 GB)

# Inicia jinx
jinx
```

El primer arranque tarda ~30 segundos mientras `uv` instala las dependencias del agente en un entorno aislado. Los arranques posteriores son instantáneos.

## Configuración del modelo

Al ejecutar la app por primera vez se crea el archivo de configuración automáticamente:

- **macOS:** `~/Library/Application Support/jinx/config.toml`
- **Linux:** `~/.config/jinx/config.toml`
- **Windows:** `%APPDATA%\jinx\config.toml`

```toml
# Proveedor activo: "local" (Ollama, sin envío de datos) o "remote" (Amazon Bedrock)
provider = "local"

[local]
# Modelos con soporte de tool calling: llama3.1, llama3.2, qwen3
model = "llama3.2:3b"
host  = "http://localhost:11434"

[remote]
# ID del modelo en Amazon Bedrock
# Ejemplo: "anthropic.claude-3-5-sonnet-20241022-v2:0"
model_id = ""
```

Edita el archivo y reinicia la app para aplicar los cambios.

## Configuración con Amazon Bedrock

Para usar modelos en AWS en lugar de Ollama:

**1. Autentícate con AWS CLI v2** (versión 2.32.0 o superior):

```bash
aws login
```

Esto abre el navegador para iniciar sesión con tus credenciales de consola AWS. Las credenciales temporales se guardan automáticamente y se renuevan durante hasta 12 horas. Consulta la [documentación oficial](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-sign-in.html) para más detalles.

**2. Activa el modelo en Bedrock** (Amazon Bedrock → Model access) en la región que vayas a usar.

**3. Edita el archivo de configuración** de jinx:

```toml
provider = "remote"

[remote]
model_id = "anthropic.claude-3-5-sonnet-20241022-v2:0"
```

Reinicia jinx para aplicar los cambios.

## Compilar desde el código fuente

Si prefieres compilar tú mismo:

```bash
# Requisitos: Rust (https://rustup.rs), uv (https://astral.sh/uv), Ollama

git clone https://github.com/ramtoearth/jinx
cd jinx
cargo install --path tui
```

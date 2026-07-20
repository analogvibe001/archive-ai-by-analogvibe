# Archive AI — versión Tauri

Puerto completo de Archive AI V2 a [Tauri](https://tauri.app) v2. La lógica de
negocio (`src/core/*.js` en la app original) fue **reescrita en Rust puro**
(`src-tauri/core-logic/`), y el frontend (`src/index.html`) es el mismo
HTML/CSS/JS de siempre — solo se reemplazó la capa de transporte: en vez de
`fetch()` a un servidor Node local y Server-Sent Events, ahora usa
`invoke()`/`listen()` de Tauri directamente (IPC nativo, sin servidor HTTP
intermedio, sin depender de que el usuario tenga Node.js instalado).

## Qué cambió respecto a la versión anterior

- **Sin Node.js en runtime.** La app anterior dependía de que el usuario
  tuviera Node instalado (de ahí todo el código del launcher buscando
  `/opt/homebrew/bin/node`, nvm, volta, etc., y el diálogo de error si no lo
  encontraba). Esta versión es un binario nativo compilado; nada que instalar.
- **Datos escribibles movidos fuera del bundle.** La versión anterior escribía
  `data/brands.json` y `templates/*.json` dentro de `Contents/Resources` del
  propio `.app` — fragil una vez que la app esté firmada/notarizada. Ahora
  esos archivos viven en `~/Library/Application Support/com.archiveai.app/`
  (se copian ahí automáticamente la primera vez que se abre la app) y son
  completamente editables.
- **Logs y el registro de deshacer** se mantienen en las mismas rutas de
  siempre (`~/.archive-ai-v2-logs/` y `~/.archive-ai-v2-undo.json`), así que
  si reemplazas la app anterior por esta, el historial y el "deshacer"
  pendiente se conservan.
- El selector de carpetas (`osascript "choose folder"`), abrir en Finder
  (`open`/`open -R`) y todo el flujo de montaje SMB (`open smb://...` +
  sondeo de `/Volumes`) se portaron tal cual a Rust — siguen siendo
  específicos de macOS, tal como pediste (no se agregó soporte Windows/Linux).

## Estructura

```
archive-ai-tauri/
├── src/index.html              ← frontend (sin cambios de UI, solo transporte)
├── src-tauri/
│   ├── core-logic/             ← crate Rust puro con toda la lógica de negocio
│   │   └── src/{scanner,date_extractor,brand_detector,template_engine,
│   │              project_analyzer,smb_resolver,execution_engine,logger}.rs
│   ├── src/{main,commands,state}.rs   ← capa Tauri (comandos + wiring)
│   ├── templates/*.json, data/brands.json   ← valores por defecto empaquetados
│   ├── icons/icon.icns
│   ├── capabilities/default.json
│   └── tauri.conf.json
└── README.md
```

## Compilar (hazlo en tu Mac — no se puede compilar un `.app` de macOS desde
   este entorno de trabajo en la nube)

### 1. Requisitos, una sola vez

```bash
xcode-select --install                 # Command Line Tools, si no las tienes
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust
cargo install tauri-cli --version "^2.0.0" --locked
```

### 2. Verificar que la lógica de negocio compila (rápido, sin GUI)

No tuve forma de compilar Rust en este entorno (sin acceso a red para
instalar el toolchain), así que el código no pasó por un compilador real
todavía. Antes de intentar compilar la app completa, corre esto — es rápido
y aísla cualquier error de sintaxis al crate de lógica pura, sin depender de
Tauri/WebKit:

```bash
cd "archive-ai-tauri/src-tauri/core-logic"
cargo check
```

Si algo no compila, el error señalará el archivo y línea exactos — avísame y
lo corrijo.

### 3. Probar en modo desarrollo

```bash
cd archive-ai-tauri
cargo tauri dev
```

### 4. Compilar el `.app` / `.dmg` final

```bash
cd archive-ai-tauri
cargo tauri build
```

El resultado queda en:
- `src-tauri/target/release/bundle/macos/Archive AI.app`
- `src-tauri/target/release/bundle/dmg/Archive AI_2.0.0_*.dmg`

### Si el build se queja del ícono

Solo incluí `icon.icns` (copiado del `.app` anterior). Si el bundler pide un
set completo de PNGs, genera uno desde el propio `.icns`:

```bash
cd src-tauri/icons
iconutil -c iconset icon.icns
sips -z 1024 1024 icon.iconset/icon_512x512@2x.png --out icon-1024.png
cargo tauri icon icon-1024.png
```

## Limitación conocida

El drag-and-drop de una carpeta hacia la ventana ya no puede leer la ruta
real del sistema de archivos directamente desde el evento de drop (eso
dependía de `File.path`, una extensión no estándar que solo exponían
Chrome/Electron; el WebView nativo de macOS que usa Tauri no la tiene). El
código ya contemplaba este caso — cuando no puede leer la ruta real, abre el
selector nativo de carpetas automáticamente, así que el flujo sigue
funcionando, solo con un clic adicional en vez de sentirse 100% "soltar y
listo". Si te importa recuperar el comportamiento exacto, se puede lograr con
la API de drag-and-drop nativa de Tauri (`onDragDropEvent`); no lo implementé
para mantener el frontend idéntico al original.

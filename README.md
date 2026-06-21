# unityfs-cli

A CLI tool to extract assets and dump metadata from Unity asset bundles.

## Features

- **Asset Extraction**:
  - `AudioClip` -> `.ogg` / `.wav` / `.mp3` / etc.
  - `Mesh` -> `.obj`
  - `Shader` -> `.shader`
  - `TextAsset` -> `.txt`
  - `Texture2D` -> `.png`
  - `VideoClip` -> `.mp4` / etc.
- **Metadata Extraction (`--metadata`)**:
  - Dumps structural JSON data for `GameObject`, `Transform`, `Material`, `MonoBehaviour`, and `AnimationClip` types.

## Usage

### Basic Extraction

```bash
unityfs-cli <PATH_TO_BUNDLE>
```

### Command-line Options

- `-o`, `--output <DIR>`: Specifies the output root directory (Default: `./out`).
- `-m`, `--metadata`: Enables JSON metadata dump for GameObjects, Transforms, Materials, MonoBehaviours, and AnimationClips.
- `-n`, `--name <NAME>`: Filters assets to extract by name (case-insensitive substring match).
- `-t`, `--type <TYPE>`: Filters assets to extract by type (case-insensitive substring match, e.g., `texture2d`, `audioclip`).
- `-b`, `--by-file`: Extracts assets directly into subdirectories named after each bundle file, without creating separate asset class subfolders (e.g., `out/bundle_name/file.png` instead of `out/Texture2D/file.png`).

### Examples

```bash
# Extract only Texture2D files containing "character" in their name:
unityfs-cli -t texture2d -n character path/to/bundle.ab

# Extract all media and dump structural JSON metadata:
unityfs-cli --metadata path/to/bundle.ab
```

### Interactive Mode

Launch the interactive terminal window by running the CLI without passing any input paths:

```bash
unityfs-cli
```

Drag and drop your file or folder directly into the terminal window and press `Enter` to extract. Type `q` or `exit` to close the interactive session.

## Building the CLI

### Standard Build

```bash
# Debug build (faster compilation)
cargo build

# Release build (highly optimized)
cargo build --release
```

### SIMD Hardware Acceleration Build (Highly Recommended)

To maximize decompression and extraction performance, it is recommended to compile the binary with native target CPU instruction sets enabled (e.g., AVX2 or NEON):

- **Windows (PowerShell):**

  ```powershell
  $env:RUSTFLAGS="-C target-cpu=native"
  cargo build --release
  ```

- **Linux / macOS (Bash):**
  ```bash
  RUSTFLAGS="-C target-cpu=native" cargo build --release
  ```

## License

[MIT License](https://github.com/lmmtrr/unityfs-cli/blob/main/LICENSE)
